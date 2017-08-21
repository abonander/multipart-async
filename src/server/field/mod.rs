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

use helpers::*;

use MyRc;

use self::httparse::EMPTY_HEADER;

mod collect;
mod headers;

pub use self::headers::FieldHeaders;

use self::collect::CollectStr;

const MAX_BUF_LEN: usize = 1024;
const MAX_HEADERS: usize = 4;

pub struct ReadFields {
    state: State
}

#[derive(Debug)]
enum State {
    NextField(ReadHeaders),
    OnField(MyRc<FieldHeaders>, Option<CollectStr>),
}

impl ReadFields {
    pub fn next_field(&mut self) {
        self.state = State::NextField(Default::default());
    }

    pub fn poll_field<'a, S: Stream + 'a>(&'a mut self, stream: &'a mut BoundaryFinder<S>) where S::Item: BodyChunk, S::Error: StreamError {

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
}


pub struct Field<'a, S: Stream + 'a> {
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

    pub fn read_string(&mut self, limit: Option<u64>) -> Poll<String, S::Error> {
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
