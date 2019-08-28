// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Poll, Stream, TryStream};

use std::rc::Rc;
use std::str;

use crate::server::boundary::BoundaryFinder;

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

pub(super) fn new_field<S: TryStream>(
    headers: FieldHeaders,
    stream: Pin<&mut PushChunk<BoundaryFinder<S>, S::Ok>>,
) -> Field<S> {
    let headers = Rc::new(headers);

    Field {
        headers: headers.clone(),
        data: FieldData { headers, stream },
        _priv: (),
    }
}

/// A single field in a multipart stream.
///
/// The data of the field is provided as a `Stream` impl in the `data` field.
///
/// To avoid the next field being initialized before this one is done being read
/// (in a linear stream), only one instance per `Multipart` instance is allowed at a time.
/// A `Drop` implementation on `FieldData` is used to notify `Multipart` that this field is done
/// being read, thus:
///
/// ### Warning About Leaks
/// If this value or the contained `FieldData` is leaked (via `mem::forget()` or some
/// other mechanism), then the parent `Multipart` will never be able to yield the next field in the
/// stream. The task waiting on the `Multipart` will also never be notified, which, depending on the
/// event loop/reactor/executor implementation, may cause a deadlock.
pub struct Field<'a, S: TryStream + 'a> {
    /// The headers of this field, including the name, filename, and `Content-Type`, if provided.
    pub headers: Rc<FieldHeaders>,
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
    headers: Rc<FieldHeaders>,
    stream: Pin<&'a mut PushChunk<BoundaryFinder<S>, S::Ok>>,
}

impl<'a, S: TryStream + 'a> FieldData<'a, S> {
    unsafe_unpinned!(stream: Pin<&'a mut PushChunk<BoundaryFinder<S>, S::Ok>>);
}

impl<S: TryStream> Stream for FieldData<'_, S>
where
    S::Ok: BodyChunk,
    S::Error: StreamError,
{
    type Item = Result<S::Ok, S::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        self.stream().as_mut().poll_next(cx)
    }
}
