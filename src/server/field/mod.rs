// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Future, Stream, Async, Poll};
use futures::Async::*;

use mime::{self, Mime};

use std::rc::Rc;
use std::{io, mem, str};

use server::boundary::BoundaryFinder;
use server::{Internal, BodyChunk, StreamError, Multipart, httparse, twoway};

use helpers::*;


use self::httparse::EMPTY_HEADER;

mod collect;
mod headers;

pub use self::headers::{FieldHeaders, ReadHeaders};

pub use self::collect::{ReadFieldText, TextField};

pub fn new_field<S: Stream>(headers: FieldHeaders, internal: Rc<Internal<S>>) -> Field<S> {
    let headers = Rc::new(headers);

    Field {
        headers: headers.clone(),
        data: FieldData {
            headers, internal
        }
    }
}

pub struct Field<S: Stream> {
    /// The headers of this field, including the name, filename, and `Content-Type`.
    pub headers: Rc<FieldHeaders>,
    pub data: FieldData<S>,
}

pub struct FieldData<S: Stream> {
    headers: Rc<FieldHeaders>,
    internal: Rc<Internal<S>>,
}

impl<S: Stream> FieldData<S> where S::Item: BodyChunk, S::Error: StreamError {

    /// Indicate that the user is done reading this field.
    pub fn done(self) { drop(self) }

    /// Get a `Future` which attempts to read the field data to a string.
    ///
    /// If `limit` is supplied, it places a size limit in bytes on the total size of the string.
    /// If an incoming chunk is expected to push the string over this limit, an error is returned.
    /// The limit value can be changed on `ReadFieldText` if desired.
    ///
    /// ### Charset
    /// For simplicity, the default UTF-8 character set is assumed, as defined in
    /// [RFC 7578 Section 5.1.2](https://tools.ietf.org/html/rfc7578#section-5.1.2).
    /// If the field body cannot be decoded as UTF-8, an error is returned.
    ///
    /// If you want to decode text in a different charset, you will need to implement it yourself.
    pub fn read_text(self, limit: Option<usize>) -> ReadFieldText<S> {
        if let Some(ref cont_type) = self.headers.cont_type {
            if cont_type.type_() != mime::TEXT {
                warn!("attempting to collect a non-text field {:?} to a string",
                      self.headers);
            }
        }

        collect::read_text(self, limit)
    }
}

impl<S: Stream> Stream for FieldData<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.internal.stream_mut().body_chunk()
    }
}

impl<S: Stream> Drop for FieldData<S> {
    fn drop(&mut self) {
        self.internal.on_field.set(false);
        self.internal.notify_task();
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
