// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Future, Stream, Async, Poll};
use futures::Async::*;

use mime::{self, Mime};

use std::{io, mem, str};

use server::boundary::BoundaryFinder;
use server::{BodyChunk, StreamError, Multipart, httparse, twoway};

use helpers::*;

use MyRc;

use self::httparse::EMPTY_HEADER;

mod collect;
mod headers;

pub use self::headers::FieldHeaders;

use self::collect::CollectStr;
use self::headers::ReadHeaders;

#[derive(Debug)]
pub struct ReadFields {
    state: State
}

#[derive(Debug)]
enum State {
    NextField(ReadHeaders),
    OnField(MyRc<FieldHeaders>, Option<CollectStr>),
}

impl ReadFields {
    pub fn new() -> Self {
        ReadFields {
            state: State::NextField(ReadHeaders::default())
        }
    }

    pub fn next_field(&mut self) {
        self.state = State::NextField(Default::default());
    }

    pub fn poll_field<'a, S: Stream + 'a>(&'a mut self, stream: &'a mut BoundaryFinder<S>) -> PollOpt<Field<'a, S>, S::Error> where S::Item: BodyChunk, S::Error: StreamError {
        loop {
            let headers = match self.state {
                State::NextField(ref mut read_hdrs) => try_ready!(read_hdrs.read_headers()),
                State::OnField()
            }
        }
    }

    fn collect_str(&mut self) -> &mut CollectStr {
        loop {
            match self.state {
                State::OnField(_, Some(ref mut collect)) => return collect,
                State::OnField(_, ref mut opt) => *opt = Some(CollectStr::default()),
                _ => panic!("Invalid state for collecting string: {:?}", self.state),
            }
        }
    }

    fn headers(&self) -> &FieldHeaders {
        match self.state {
            State::OnField(ref headers, _) => return headers,
            _ => panic!("Invalid state for collecting string: {:?}", self.state),
        }
    }
}


pub struct Field<'a, S: Stream + 'a> {
    /// The headers of this field, including the name, filename, and `Content-Type`.
    ///
    /// If the `use_arc` feature is set, the `MyRc` type is `Arc` for sharing across threads.
    /// Otherwise, it is `Rc` for cheaper, non-atomic refcount updates.
    pub headers: MyRc<FieldHeaders>,
    pub data: FieldData<'a, S>,
}

pub struct FieldData<'a, S: Stream + 'a> {
    stream: &'a mut BoundaryFinder<S>,
    fields: &'a mut ReadFields,
}

impl<'a, S: Stream + 'a> FieldData<'a, S> where S::Item: BodyChunk, S::Error: StreamError {
    /// Attempt to read the next field in the stream, discarding the remaining data in this field.
    pub fn next_field(self) -> Poll<Option<Field<'a, S>>, S::Error> {
        self.fields.poll_field(self.stream)
    }

    /// Attempt to read the field data to a string.
    ///
    /// If the string could not be read all in one go, the intermediate result is saved internally.
    /// This method is meant to be called repeatedly until it yields a `String`. If called again
    /// afterwards, returns an empty string.
    ///
    /// If `limit` is supplied, it places a size limit in bytes on the total size of the string.
    /// If an incoming chunk is expected to push the string over this limit, an error is returned.
    /// The latest value for `limit` is always used.
    ///
    /// ### Charset
    /// For simplicity, the default UTF-8 character set is assumed, as defined in
    /// [RFC 7578 Section 5.1.2](https://tools.ietf.org/html/rfc7578#section-5.1.2).
    /// If the field's `Content-Type` header contains a `charset` param that is *not* `UTF-8`,
    /// or if the field body could not be decoded as UTF-8, an error is returned.
    ///
    /// If you want to decode text in a different charset, you will need to implement it yourself.
    pub fn read_string(&mut self, limit: Option<usize>) -> Poll<String, S::Error> {
        if let Some(cont_type) = self.fields.headers().cont_type {
            if cont_type.type_() != mime::TEXT {
                warn!("attempting to collect a non-text field {:?} to a string",
                      self.fields.headers());
            }

            if let Some(charset) = cont_type.get_param(mime::CHARSET) {
                if charset != mime::UTF_8 {
                    return error(format!("unsupported charset ({}) for field {:?}", charset,
                                            self.fields.headers()));
                }
            }
        }

        let collect = self.fields.collect_str();
        collect.limit = limit;
        collect.collect(self.stream)
    }
}

impl<'s, S: Stream + 's> Stream for FieldData<'s, S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.stream.body_chunk()
    }
}

#[test]
fn test_header_end_split() {
    assert_eq!(header_end_split(b"\r\n\r", b"\n"), Some(1));
    assert_eq!(header_end_split(b"\r\n", b"\r\n"), Some(2));
    assert_eq!(header_end_split(b"\r", b"\n\r\n"), Some(3));
    assert_eq!(header_end_split(b"\r\n\r\n", b"FOOBAR"), None);
    assert_eq!(header_end_split(b"FOOBAR", b"\r\n\r\n"), None);
}
