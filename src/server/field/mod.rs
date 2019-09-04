// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Poll, Stream, TryStream};

use std::rc::Rc;
use std::str;

use super::boundary::BoundaryFinder;
use super::Multipart;

use std::fmt;

use crate::{BodyChunk, StreamError};

// mod collect;
mod headers;

pub use self::headers::FieldHeaders;
pub(crate) use self::headers::ReadHeaders;

//pub use self::collect::{ReadTextField, TextField};
use futures::task::Context;
use std::pin::Pin;
use crate::server::PushChunk;
use futures::future::Ready;
use futures_test::futures_core_reexport::Future;

pub struct NextField<'a, S: TryStream + 'a> {
    multipart: Option<Pin<&'a mut Multipart<S>>>,
    has_next_field: bool,
}

impl<'a, S: 'a> NextField<'a, S> {
    pub(crate) fn new(multipart: Pin<&'a mut Multipart<S>>) {
        NextField {
            multipart,
            has_next_field: false,
        }
    }
}

impl<'a, S: 'a> Future for NextField<'a, S> where S: TryStream, S::Ok: BodyChunk, S::Error: StreamError {
    type Output = Option<Result<Field<'a, S>, S::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        use PollState::*;

        // `false` and `multipart = Some` means we haven't polled for next field yet
        self.has_next_field = self.has_next_field
            || ready!(self.multipart.as_pin_mut()?.poll_has_next_field(cx)?);

        if !self.has_next_field {
            // end of stream, next `?` will return
            self.multipart = None;
        }

        Ready(Some(Field {
            headers: ready!(self.multipart.as_pin_mut()?.poll_field_headers(cx)?),
            data: FieldData {
                multipart: self.multipart.take()?
            },
            _priv: ()
        }))
    }
}

/// A single field in a multipart stream.
///
/// The data of the field is provided as a `Stream` impl in the `data` field.
pub struct Field<'a, S: TryStream + 'a> {
    /// The headers of this field, including the name, filename, and `Content-Type`, if provided.
    pub headers: FieldHeaders,
    /// The data of this field in the request, represented as a stream of chunks.
    pub data: FieldData<'a, S>,
    _priv: (),
}

impl<S: TryStream> fmt::Debug for Field<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Field")
            .field("headers", &self.headers)
            .field("data", &"<FieldData>")
            .finish()
    }
}

/// The data of a field in a multipart stream, as a stream of chunks.
///
/// It may be read to completion via the `Stream` impl, or collected to a string with `read_text()`.
pub struct FieldData<'a, S: TryStream + 'a> {
    multipart: Pin<&'a mut Multipart<S>>,
}

impl<S: TryStream> Stream for FieldData<'_, S>
where
    S::Ok: BodyChunk,
    S::Error: StreamError,
{
    type Item = Result<S::Ok, S::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        self.multipart.as_mut().poll_field_chunk(cx)
    }
}
