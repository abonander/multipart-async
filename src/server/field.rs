// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Future, Stream, Async, Poll};
use futures::Async::*;

use mime::Mime;

use std::{io, mem, str};

use server::boundary::BoundaryFinder;
use server::{BodyChunk, StreamError, Multipart, httparse, twoway};

use self::httparse::EMPTY_HEADER;

use helpers::*;

const MAX_BUF_LEN: usize = 1024;
const MAX_HEADERS: usize = 4;

pub struct NextField<S: Stream> {
    stream: SomeCell<BoundaryFinder<S>>,
    accumulator: Vec<u8>,
    headers: Option<Headers>,
}

pub fn next_field<S: Stream>(stream: BoundaryFinder<S>) -> NextField<S> {
    NextField {
        stream: SomeCell::new(stream, "NextField value already taken"),
        accumulator: Vec::new(),
        headers: None,
    }
}

impl<S: Stream> NextField<S> where S::Item: BodyChunk, S::Error: StreamError {
    fn read_headers(&mut self) -> Poll<Headers, S::Error> {
        if let Some(headers) = self.headers.take() {
            return ready(headers);
        }

        loop {
            let chunk = try_ready!(self.stream.poll())
                .ok_or_else(|| io_error("unexpected end of stream"))?;

            // End of the headers section is signalled by a double-CRLF
            if let Some(header_end) = twoway::find_bytes(chunk.as_slice(), b"\r\n\r\n") {
                // Split after the double-CRLF because we don't want to yield it and httparse expects it
                let (headers, rem) = chunk.split_at(header_end + 4);
                self.stream.try_as_mut()?.push_chunk(rem);

                if !self.accumulator.is_empty() {
                    self.accumulator.extend_from_slice(headers.as_slice());
                    let headers = parse_headers(&self.accumulator)?;
                    self.accumulator.clear();

                    break ready(headers);
                } else {
                    break ready(parse_headers(headers.as_slice())?);
                }
            } else if let Some(split_idx) = header_end_split(&self.accumulator, chunk.as_slice()) {
                let (head, tail) = chunk.split_at(split_idx);
                self.accumulator.extend_from_slice(head.as_slice());
                continue;
            }

            // TODO: edge case where double-CRLF falls on chunk boundaries

            if self.accumulator.len().saturating_add(chunk.len()) > MAX_BUF_LEN {
                return error("headers section too long or trailing double-CRLF missing");
            }

            self.accumulator.extend_from_slice(chunk.as_slice());
        }
    }

    fn read_to_string(&mut self) -> Poll<String, S::Error> {
        loop {
            if self.accumulator.is_empty() {
                self.accumulator = try_ready!(self.stream.poll())
                    .map_or(Vec::new(), BodyChunk::into_vec);
            }

            match try_ready!(self.stream.poll()) {
                Some(chunk) => self.accumulator.extend_from_slice(chunk.as_slice()),
                None => return ready(String::from_utf8(self.take_accumulator())?),
            }
         }
    }

    fn take_accumulator(&mut self) -> Vec<u8> {
        mem::replace(&mut self.accumulator, Vec::new())
    }
}
impl<S: Stream> Future for NextField<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = Field<S>;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let headers = try_ready!(self.read_headers());

        let data = if let Some(cont_type) = headers.cont_type {
            FieldData::File(FileData {
                content_type: cont_type,
                filename: headers.filename,
                multipart: Multipart { stream: self.stream.try_take()? },
                _priv: (),
            })
        } else if let Ready(text) = self.read_to_string()? {
            FieldData::Text(
                TextData {
                    text: text,
                    multipart: Multipart { stream: self.stream.try_take()? },
                    _priv: (),
                }
            )
        } else {
            self.headers = Some(headers);
            return not_ready();
        };

        ready(Field {
            name: headers.name,
            data: data,
            _priv: (),
        })
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

fn parse_headers(bytes: &[u8]) -> io::Result<Headers> {
    debug_assert!(bytes.ends_with(b"\r\n\r\n"),
                  "header byte sequence does not end with `\\r\\n\\r\\n`: {}",
                  show_bytes(bytes));

    unimplemented!()
}
    let mut header_buf = [EMPTY_HEADER; MAX_HEADERS];

    let (_, headers) = httparse::parse_headers(bytes, &mut header_buf).map_err(io_error)?;

    let mut out_headers = Headers::default();

    for header in headers {
        let str_val = str::from_utf8(header.value)
            .map_err(|_| io_error("multipart field headers must be UTF-8 encoded"))?
            .trim();

        match header.name {
            "Content-Disposition" => parse_cont_disp_val(str_val, &mut out_headers)?,
            "Content-Type" => out_headers.cont_type = Some(str_val.parse::<Mime>().map_err(io_error)),
        }
    }

    Ok(out_headers)
}

fn parse_cont_disp_val(val: &str, out: &mut Headers) -> io::Result<()> {
    // Only take the first section, the rest can be in quoted strings that we want to handle
    let mut sections = val.splitn(';', 1).map(str::trim);

    match sections.next() {
        Some("form-data") => (),
        Some(other) => error(format!("unexpected multipart field Content-Disposition: {}", other))?,
        None => error("each multipart field requires a Content-Disposition: form-data header")?,
    }

    let keyvals = sections.next().unwrap_or("");

    for section in sections {
        if section.starts_with("name") {
            out.name =
        }
    }

    if out.name.is_empty() {
        return error(format!("expected 'name' attribute in Content-Disposition: {}", val));
    }

    Ok(())
}

fn collect_param_val

#[derive(Default)]
struct Headers {
    name: String,
    cont_type: Option<Mime>,
    filename: Option<String>,
}

pub struct Field<S: Stream> {
    pub name: String,
    pub data: FieldData<S>,
    _priv: (),
}

pub enum FieldData<S: Stream> {
    Text(TextData<S>),
    File(FileData<S>),
}

pub struct TextData<S: Stream> {
    pub text: String,
    pub multipart: Multipart<S>,
    _priv: (),
}

pub struct FileData<S: Stream> {
    pub filename: Option<String>,
    pub content_type: Mime,
    pub multipart: Multipart<S>,
    _priv: (),
}

const CONT_DISP: &str = "Content-Disposition";

#[test]
fn test_header_end_split() {
    assert_eq!(header_end_split(b"\r\n\r", b"\n"), Some(1));
    assert_eq!(header_end_split(b"\r\n", b"\r\n"), Some(2));
    assert_eq!(header_end_split(b"\r", b"\n\r\n"), Some(3));
    assert_eq!(header_end_split(b"\r\n\r\n", b"FOOBAR"), None);
    assert_eq!(header_end_split(b"FOOBAR", b"\r\n\r\n"), None);
}
