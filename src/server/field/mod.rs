// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures_core::{Future, Poll, Stream, TryStream};
use std::task::Poll::*;

use std::rc::Rc;
use std::str;

use super::boundary::BoundaryFinder;
use super::Multipart;

use std::fmt;

use crate::BodyChunk;

// mod collect;
mod headers;

pub use self::headers::FieldHeaders;
pub(crate) use self::headers::ReadHeaders;

//pub use self::collect::{ReadTextField, TextField};
use futures_core::task::Context;
use std::pin::Pin;
use crate::server::{PushChunk, Error};

/// A `Future` potentially yielding the next field in the multipart stream.
///
/// If there are no more fields in the stream, `Ok(None)` is returned.
///
/// See [`Multipart::next_field()`](../struct.Multipart.html#method.next_field) for usage.
pub struct NextField<'a, S: TryStream + 'a> {
    multipart: Option<Pin<&'a mut Multipart<S>>>,
    has_next_field: bool,
}

impl<'a, S: TryStream + 'a> NextField<'a, S> {
    pub(crate) fn new(multipart: Pin<&'a mut Multipart<S>>) -> Self {
        NextField {
            multipart: Some(multipart),
            has_next_field: false,
        }
    }

    fn multipart(&mut self) -> Option<Pin<&mut Multipart<S>>> {
        Some(self.multipart.as_mut()?.as_mut())
    }
}

impl<'a, S: 'a> Future for NextField<'a, S> where S: TryStream, S::Ok: BodyChunk, Error<S::Error>: From<S::Error> {
    type Output = super::Result<Option<Field<'a, S>>, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // since we can't use `?` with `Option<...>` in this context
        macro_rules! multipart {
            (get) => {
                if let Some(ref mut multipart) = self.multipart {
                    multipart.as_mut()
                } else {
                    return Ready(Ok(None));
                }
            };
            (take) => {
                if let Some(multipart) = self.multipart.take() {
                    multipart
                } else {
                    return Ready(Ok(None));
                }
            }
        }

        // `false` and `multipart = Some` means we haven't polled for next field yet
        self.has_next_field = self.has_next_field
            || ready!(multipart!(get).poll_has_next_field(cx)?);

        if !self.has_next_field {
            // end of stream, next `?` will return
            self.multipart = None;
        }

        Ready(Ok(Some(Field {
            headers: ready!(multipart!(get).poll_field_headers(cx)?),
            data: FieldData {
                multipart: multipart!(take)
            },
            _priv: ()
        })))
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
    Error<S::Error>: From<S::Error>
{
    type Item = super::Result<S::Ok, S::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        self.multipart.as_mut().poll_field_chunk(cx)
    }
}
