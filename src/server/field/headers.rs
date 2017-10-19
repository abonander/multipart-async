use futures::Stream;

use http::header::{HeaderMap, HeaderName, HeaderValue};

use mime::{self, Mime, Name};

use std::ascii::AsciiExt;
use std::str;

use server::{httparse, twoway};
use server::boundary::BoundaryFinder;

use { BodyChunk, StreamError};

use self::httparse::{EMPTY_HEADER, Status};

use helpers::*;

const MAX_BUF_LEN: usize = 1024;
const MAX_HEADERS: usize = 4;

/// The headers of a `Field`, including the name, filename, and `Content-Type`, if provided.
///
/// ### Note: Untrustworthy
/// These values are provided directly by the client, and as such, should be considered
/// *untrustworthy* and potentially **dangerous**. Avoid any unsanitized usage on the filesystem
/// or in a shell or database, or performing unsafe operations with the assumption of a
/// certain file type. Sanitizing/verifying these values is (currently) beyond the scope of this
/// crate.
#[derive(Clone, Default, Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct FieldHeaders {
    /// The name of the field as provided by the client.
    ///
    /// ### Special Value: `_charset_`
    /// If the client intended a different character set than UTF-8 for its text values, it may
    /// provide the name of the charset as a text field (ASCII-encoded) with the name `_charset_`.
    /// See [IETF RFC 7578, Section 4.6](https://tools.ietf.org/html/rfc7578#section-4.6) for more.
    ///
    /// Alternately, the charset can be provided for an individual field as a `charset` parameter
    /// to its `Content-Type` header; see the `charset()` method for a convenient wrapper.
    pub name: String,
    /// The name of the file as it was on the client. If not provided, it may still have been a
    /// file field.
    pub filename: Option<String>,
    /// The `Content-Type` of this field, as provided by the client. If `None`, then the field
    /// is probably text, but this is not guaranteed.
    pub content_type: Option<Mime>,
    /// Any additional headers, standard or otherwise, for this field as provided by the client.
    ///
    /// The size of this map will be limited internally.
    pub ext: HeaderMap,
}

impl FieldHeaders {
    /// `true` if `content_type` is `None` or `text/*` (such as `text/plain`).
    ///
    /// **Note**: this does not guarantee that the field data is compatible with
    /// `FieldData::read_text()`; supporting more encodings than ASCII/UTF-8 is (currently)
    /// beyond the scope of this crate.
    pub fn is_text(&self) -> bool {
        self.content_type.as_ref().map_or(true, |ct| ct.type_() == mime::TEXT)
    }

    /// The character set of this field, if provided.
    pub fn charset(&self) -> Option<Name> {
        self.content_type.as_ref().and_then(|ct| ct.get_param(mime::CHARSET))
    }
}

#[derive(Debug, Default)]
pub struct ReadHeaders {
    accumulator: Vec<u8>
}

impl ReadHeaders {
    pub fn read_headers<S: Stream>(&mut self, stream: &mut BoundaryFinder<S>) -> PollOpt<FieldHeaders, S::Error>
    where S::Item: BodyChunk, S::Error: StreamError {
        loop {
            trace!("read_headers state: accumulator: {}", show_bytes(&self.accumulator));

            let chunk = match try_ready!(stream.poll()) {
                Some(chunk) => chunk,
                None => return if !self.accumulator.is_empty() {
                    error("unexpected end of stream")
                } else {
                    trace!("end of request reached");
                    ready(None)
                },
            };

            trace!("got chunk for headers: {}", show_bytes(chunk.as_slice()));

            // End of the headers section is signalled by a double-CRLF
            if let Some(header_end) = twoway::find_bytes(chunk.as_slice(), b"\r\n\r\n") {
                // Split after the double-CRLF because we don't want to yield it and httparse expects it
                let (headers, rem) = chunk.split_at(header_end + 4);
                stream.push_chunk(rem);

                if !self.accumulator.is_empty() {
                    self.accumulator.extend_from_slice(headers.as_slice());
                    let headers = parse_headers(&self.accumulator)?;
                    self.accumulator.clear();

                    return ready(Some(headers));
                } else {
                    return ready(Some(parse_headers(headers.as_slice())?));
                }
            } else if let Some(split_idx) = header_end_split(&self.accumulator, chunk.as_slice()) {
                let (head, tail) = chunk.split_at(split_idx);
                self.accumulator.extend_from_slice(head.as_slice());
                stream.push_chunk(tail);
                continue;
            }

            if self.accumulator.len().saturating_add(chunk.len()) > MAX_BUF_LEN {
                return error("headers section too long or trailing double-CRLF missing");
            }

            self.accumulator.extend_from_slice(chunk.as_slice());
        }
    }
}

const CRLF2: &[u8] = b"\r\n\r\n";

/// Check if the double-CRLF falls between chunk boundaries, and if so, the split index of
/// the second boundary
fn header_end_split(first: &[u8], second: &[u8]) -> Option<usize> {
    fn split_subcheck(start: usize, first: &[u8], second: &[u8]) -> bool {
        first.len() >= start && first[first.len() - start ..].iter().chain(second).take(4).eq(CRLF2)
    }

    if split_subcheck(3, first, second) {
        Some(1)
    } else if split_subcheck(2, first, second) {
        Some(2)
    } else if split_subcheck(1, first, second) {
        Some(3)
    } else {
        None
    }
}

fn parse_headers<E: StreamError>(bytes: &[u8]) -> Result<FieldHeaders, E> {
    debug_assert!(bytes.ends_with(b"\r\n\r\n"),
                  "header byte sequence does not end with `\\r\\n\\r\\n`: {}",
                  show_bytes(bytes));

    let mut header_buf = [EMPTY_HEADER; MAX_HEADERS];

    let headers = match httparse::parse_headers(bytes, &mut header_buf) {
        Ok(Status::Complete((_, headers))) => headers,
        Ok(Status::Partial) => ret_err!("field headers incomplete: {}", show_bytes(bytes)),
        Err(e) => ret_err!("error parsing headers: {}; from buffer: {}", e, show_bytes(bytes)),
    };

    trace!("parsed headers: {:?}", headers);

    let mut out_headers = FieldHeaders::default();

    let mut dupe_cont_type = false;

    for header in headers {
        if "Content-Disposition".eq_ignore_ascii_case(header.name) {
            if !out_headers.name.is_empty() {
                ret_err!("duplicate `Content-Disposition` header on field: {}", out_headers.name);
            }

            let str_val = str::from_utf8(header.value)
                .or_else(|_| error("multipart `Content-Disposition` header values \
                                                     must be UTF-8 encoded"))?
                .trim();

            parse_cont_disp_val(str_val, &mut out_headers)?;
        } else if "Content-Type".eq_ignore_ascii_case(header.name) {
            if out_headers.content_type.is_some() {
                // try to get the field name from `Content-Disposition` first
                // if none was provided then that's the bigger issue
                dupe_cont_type = true;
                continue;
            }

            let str_val = str::from_utf8(header.value)
                .or_else(|_| error("multipart `Content-Type` header values \
                                                     must be UTF-8 encoded"))?
                .trim();

            out_headers.content_type = Some(
                str_val.parse::<Mime>()
                    .or_else(|_|
                        fmt_err!("could not parse MIME type from {:?}", str_val)
                    )?
            );
        } else {
            let hdr_name = HeaderName::from_bytes(header.name.as_bytes())
                .or_else(|e|
                    fmt_err!("error on multipart field header \"{}\": {}",
                             header.name, e)
                )?;

            let hdr_val = HeaderValue::from_bytes(bytes)
                .or_else(|e|
                    fmt_err!("error on multipart field header \"{}\": {}",
                             header.name, e)
                )?;

            out_headers.ext.append(hdr_name, hdr_val);
        }
    }

    if out_headers.name.is_empty() {
        // missing `name` parameter in a provided `Content-Disposition` is covered separately
        if let Some(filename) = out_headers.filename {
            ret_err!("missing `Content-Disposition` header on a field \
                      (filename: {}) in this multipart request", filename);
        }

        if let Some(content_type) = out_headers.content_type {
            ret_err!("missing `Content-Disposition` header on a field \
                      (Content-Type: {}) in this multipart request", content_type);
        }

        ret_err!("missing `Content-Disposition` header on a field in this multipart request");
    }

    if dupe_cont_type {
        ret_err!("duplicate `Content-Type` header in field: {}", out_headers.name);
    }

    Ok(out_headers)
}

fn parse_cont_disp_val<E: StreamError>(val: &str, out: &mut FieldHeaders) -> Result<(), E> {
    debug!("parse_cont_disp_val({:?})", val);

    // Only take the first section, the rest can be in quoted strings that we want to handle
    let mut sections = val.splitn(2, ';').map(str::trim);

    if !sections.next().unwrap_or("").eq_ignore_ascii_case("form-data") {
        ret_err!("unexpected/unsupported field header `Content-Disposition: {}` \
                  in this multipart request; each field must have exactly one \
                  `Content-Disposition: form-data` header with a `name` parameter", val);
    }

    let mut rem = sections.next().unwrap_or("");

    while let Some((key, val, rest)) = parse_keyval(rem) {
        rem = rest;

        match key {
            "name" => out.name = val.to_string(),
            "filename" => out.filename = Some(val.to_string()),
            _ => debug!("unknown key-value pair in Content-Disposition: {:?} = {:?}", key, val),
        }
    }

    if out.name.is_empty() {
        ret_err!("expected 'name' parameter in `Content-Disposition: {}`", val);
    }

    Ok(())
}

fn parse_keyval(input: &str) -> Option<(&str, &str, &str)> {
    if input.trim().is_empty() { return None; }

    let (name, rest) = try_opt!(param_name(input));
    let (val, rest) = try_opt!(param_val(rest));

    Some((name, val, rest))
}

fn param_name(input: &str) -> Option<(&str, &str)> {
    let mut splits = input.trim_left_matches(&[' ', ';'][..]).splitn(2, '=');

    let name = try_opt!(splits.next()).trim();
    let rem = splits.next().unwrap_or("");

    Some((name, rem))
}

fn param_val(input: &str) -> Option<(&str, &str)> {
    // continue until the opening quote or the terminating semicolon
    static QUOTE_SEMICOLON: [char; 2] = ['"', ';'];
    let mut tk_splits = input.splitn(2, &QUOTE_SEMICOLON[..]);

    let token = try_opt!(tk_splits.next()).trim();
    let rem = tk_splits.next().unwrap_or("");

    // the value doesn't have to be in quotes if it doesn't contain forbidden chars like `;`
    if !token.is_empty() {
        return Some((token, rem.trim_matches(&[' ', ';'][..])));
    }

    // continue until the terminating quote
    let mut qt_splits = rem.splitn(2, '"');

    let qstr = try_opt!(qt_splits.next()).trim();
    let rem = qt_splits.next().unwrap_or_else(|| { warn!("unterminated quote: {:?}", qstr); "" })
        .trim_matches(&[' ', ';'][..]);

    Some((qstr, rem))
}

#[test]
fn test_header_end_split() {
    assert_eq!(header_end_split(b"\r\n\r", b"\n"), Some(1));
    assert_eq!(header_end_split(b"\r\n", b"\r\n"), Some(2));
    assert_eq!(header_end_split(b"\r", b"\n\r\n"), Some(3));
    assert_eq!(header_end_split(b"\r\n\r\n", b"FOOBAR"), None);
    assert_eq!(header_end_split(b"FOOBAR", b"\r\n\r\n"), None);
}

#[test]
fn test_parse_keyval() {
    assert_eq!(
        parse_keyval("name = field; x-attr = \"some;value\"; filename = file.bin"),
        Some(("name", "field", "x-attr = \"some;value\"; filename = file.bin"))
    );

    assert_eq!(
        parse_keyval("x-attr = \"some;value\"; filename = file.bin"),
        Some(("x-attr", "some;value", "filename = file.bin"))
    );

    assert_eq!(
        parse_keyval("filename = file.bin"),
        Some(("filename", "file.bin", ""))
    );

    assert_eq!(parse_keyval(""), None);
}

#[test]
fn test_parse_headers() {
    use StringError;

    let parse_headers = parse_headers::<StringError>;

    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = \"field\"\r\n\r\n"),
        Ok(FieldHeaders { name: "field".into(), .. FieldHeaders::default()})
    );

    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = \"field\"\r\n\
                        Content-Type: application/octet-stream\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            content_type: Some(mime::APPLICATION_OCTET_STREAM),
            .. FieldHeaders::default()
        })
    );

    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = \"field\"\r\n\
                        Content-Type: text/plain; charset=\"utf-8\"\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            content_type: Some(mime::TEXT_PLAIN_UTF_8),
            .. FieldHeaders::default()
        })
    );

    // lowercase
    assert_eq!(
        parse_headers(b"content-disposition: form-data; name = \"field\"\r\n\r\n"),
        Ok(FieldHeaders { name: "field".into(), .. FieldHeaders::default()})
    );

    assert_eq!(
        parse_headers(b"content-disposition: form-data; name = \"field\"\r\n\
                        content-type: application/octet-stream\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            content_type: Some(mime::APPLICATION_OCTET_STREAM),
            .. FieldHeaders::default()
        })
    );

    // mixed case
    assert_eq!(
        parse_headers(b"cOnTent-dIsPosition: form-data; name = \"field\"\r\n\r\n"),
        Ok(FieldHeaders { name: "field".into(), .. FieldHeaders::default()})
    );

    assert_eq!(
        parse_headers(b"contEnt-disPosition: form-data; name = \"field\"\r\n\
                        coNtent-tyPe: application/octet-stream\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            content_type: Some(mime::APPLICATION_OCTET_STREAM),
            .. FieldHeaders::default()
        })
    );

    // omitted quotes
    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = field\r\n\r\n"),
        Ok(FieldHeaders { name: "field".into(), .. FieldHeaders::default()})
    );

    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = field\r\n\
                        Content-Type: application/octet-stream\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            content_type: Some(mime::APPLICATION_OCTET_STREAM),
            .. FieldHeaders::default()
        })
    );

    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = field\r\n\
                        Content-Type: text/plain; charset=utf-8\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            content_type: Some(mime::TEXT_PLAIN_UTF_8),
            .. FieldHeaders::default()
        })
    );

    // filename without quotes with extension
    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = field; filename = file.bin\r\n\
                        Content-Type: application/octet-stream\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            filename: Some("file.bin".into()),
            content_type: Some(mime::APPLICATION_OCTET_STREAM),
            .. FieldHeaders::default()
        })
    );

    // reversed headers (can happen)
    assert_eq!(
        parse_headers(b"Content-Type: application/octet-stream\r\n\
                        Content-Disposition: form-data; name = field; filename = file.bin\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            filename: Some("file.bin".into()),
            content_type: Some(mime::APPLICATION_OCTET_STREAM),
            .. FieldHeaders::default()
        })
    );

    // quoted parameter with semicolon (allowed by spec)
    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = field; x-attr = \"some;value\"; \
                        filename = file.bin\r\n\r\n"),
        Ok(FieldHeaders {
            name: "field".into(),
            filename: Some("file.bin".into()),
            content_type: None,
            .. FieldHeaders::default()
        })
    )
}

#[test]
fn test_parse_headers_errors() {
    use StringError;

    let parse_headers = parse_headers::<StringError>;

    // missing content-disposition
    assert_eq!(
        parse_headers(b"Content-Type: application/octet-stream\r\n\r\n").unwrap_err(),
        "missing `Content-Disposition` header on a field \
         (Content-Type: application/octet-stream) in this multipart request"
    );

    // duplicate content-disposition
    assert_eq!(
        parse_headers(b"Content-Disposition: form-data; name = field\r\n\
                        Content-Disposition: form-data; name = field2\r\n\r\n").unwrap_err(),
        "duplicate `Content-Disposition` header on field: field"
    );
}
