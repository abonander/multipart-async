use futures::{Poll, Stream};

use mime::Mime;

use std::{io, str};

use server::{Multipart, BodyChunk, StreamError, httparse, twoway};
use server::boundary::BoundaryFinder;

use self::httparse::EMPTY_HEADER;

use helpers::*;

const MAX_BUF_LEN: usize = 1024;
const MAX_HEADERS: usize = 4;

#[derive(Clone, Default, Debug)]
pub struct FieldHeaders {
    pub name: String,
    pub cont_type: Option<Mime>,
    pub filename: Option<String>,
}

#[derive(Debug, Default)]
pub struct ReadHeaders {
    accumulator: Vec<u8>
}

impl ReadHeaders {
    pub fn read_headers<S: Stream>(&mut self, stream: &mut BoundaryFinder<S>) -> PollOpt<FieldHeaders, S::Error>
    where S::Item: BodyChunk, S::Error: StreamError {
        loop {
            let chunk = match try_ready!(stream.poll()) {
                Some(chunk) => chunk,
                None => return if !self.accumulator.is_empty() {
                    io_error("unexpected end of stream")
                } else {
                    ready(None)
                },
            };

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

    fn take_accumulator(&mut self) -> Vec<u8> {
        replace_default(&mut self.accumulator)
    }
}

const CRLF2: &[u8] = b"\r\n\r\n";

/// Check if the double-CRLF falls between chunk boundaries, and if so, the split index of
/// the second boundary
fn header_end_split(first: &[u8], second: &[u8]) -> Option<usize> {
    fn split_subcheck(start: usize, first: &[u8], second: &[u8]) -> bool {
        first.len() >= start && first[first.len() - start ..].iter().chain(second).take(4).eq(CRLF2)
    }

    let len = first.len();

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

fn parse_headers(bytes: &[u8]) -> io::Result<FieldHeaders> {
    debug_assert!(bytes.ends_with(b"\r\n\r\n"),
                  "header byte sequence does not end with `\\r\\n\\r\\n`: {}",
                  show_bytes(bytes));

    let mut header_buf = [EMPTY_HEADER; MAX_HEADERS];

    let (_, headers) = httparse::parse_headers(bytes, &mut header_buf).map_err(io_error)?;

    let mut out_headers = FieldHeaders::default();

    for header in headers {
        let str_val = str::from_utf8(header.value)
            .map_err(|_| io_error("multipart field headers must be UTF-8 encoded"))?
            .trim();

        match header.name {
            "Content-Disposition" => parse_cont_disp_val(str_val, &mut out_headers)?,
            "Content-Type" => out_headers.cont_type = Some(str_val.parse::<Mime>().map_err(io_error)?),
        }
    }

    Ok(out_headers)
}

fn parse_cont_disp_val(val: &str, out: &mut FieldHeaders) -> io::Result<()> {
    // Only take the first section, the rest can be in quoted strings that we want to handle
    let mut sections = val.splitn(1, ';').map(str::trim);

    match sections.next() {
        Some("form-data") => (),
        Some(other) => error(format!("unexpected multipart field Content-Disposition: {}", other))?,
        None => error("each multipart field requires a Content-Disposition: form-data header")?,
    }

    let mut rem = sections.next().unwrap_or("");

    while let Some((key, val, rest)) = parse_keyval(rem) {
        rem = rest;

        match key {
            "name" => out.name = val.to_string(),
            "filename" => out.filename = Some(val.to_string()),
            other => debug!("unknown key-value pair in Content-Disposition: {:?} = {:?}", key, val),
        }
    }

    if out.name.is_empty() {
        return error(format!("expected 'name' attribute in Content-Disposition: {}", val));
    }

    Ok(())
}

fn parse_keyval(input: &str) -> Option<(&str, &str, &str)> {
    let (name, rest) = try_opt!(param_name(input));
    let (val, rest) = try_opt!(param_val(rest));

    Some((name, val, rest))
}

fn param_name(input: &str) -> Option<(&str, &str)> {
    let mut splits = input.trim_left_matches(&[' ', ';'][..]).splitn(1, '=');

    let name = try_opt!(splits.next()).trim();
    let rem = splits.next().unwrap_or("");

    Some((name, rem))
}

fn param_val(input: &str) -> Option<(&str, &str)> {
    let mut splits = input.splitn(2, &['"'][..]);

    let token = try_opt!(splits.next()).trim();

    // the value doesn't have to be in quotes if it doesn't contain forbidden chars like `;`
    if !token.is_empty() {
        let mut splits = token.splitn(1, ';');
        let token = try_opt!(splits.next()).trim();
        let rem = splits.next().unwrap_or("");

        return Some((token, rem));
    }

    let qstr = try_opt!(splits.next()).trim();
    let rem = splits.next().unwrap_or_else(|| { warn!("unterminated quote: {:?}", qstr); "" });

    Some((qstr, rem))
}
