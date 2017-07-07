// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Future, Stream, Async, Poll};
use futures::Async::*;

use mime::Mime;

use std::{io, mem};

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
            }

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

fn parse_headers(bytes: &[u8]) -> io::Result<Headers> {
        let mut header_buf = httparse::parse_headers(bytes).map_err(io_error)?;


}

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
