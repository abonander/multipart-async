// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! The server-side abstraction for multipart requests. Enabled with the `server` feature (on by
//! default).
//!
//! Use this when you are implementing an HTTP server and want to
//! to accept, parse, and serve HTTP `multipart/form-data` requests (file uploads).
//!
//! See the `Multipart` struct for more info.
extern crate httparse;
extern crate twoway;

use futures::{Poll, Stream};
use futures::task::{self, Task, Context};

use std::cell::Cell;
use std::rc::Rc;

use self::boundary::BoundaryFinder;

use {BodyChunk, StreamError};

macro_rules! try_opt (
    ($expr:expr) => (
        match $expr {
            Some(val) => val,
            None => return None,
        }
    )
);

macro_rules! ret_err (
    ($($args:tt)+) => (
            return fmt_err!($($args)+);
    )
);

macro_rules! fmt_err(
    ($string:expr) => (
        ::helpers::error($string)
    );
    ($string:expr, $($args:tt)*) => (
        ::helpers::error(format!($string, $($args)*))
    );
);

mod boundary;
mod field;

use helpers::*;

use self::field::ReadHeaders;

pub use self::field::{Field, FieldHeaders, FieldData, ReadTextField, TextField};

#[cfg(feature = "hyper")]
mod hyper;

#[cfg(feature = "hyper")]
pub use self::hyper::{MinusBody, MultipartService};
use std::pin::Pin;

/// The server-side implementation of `multipart/form-data` requests.
///
/// This will parse the incoming stream into `Field` instances via its
/// `Stream` implementation.
///
/// To maintain consistency in the underlying stream, this will not yield more than one
/// `Field` at a time. A `Drop` implementation on `FieldData` is used to signal
/// when it's time to move forward, so do avoid leaking that type or anything which contains it
/// (`Field`, `ReadTextField`, or any stream combinators).
pub struct Multipart<S: Stream> {
    inner: BoundaryFinder<S>,
    read_hdr: ReadHeaders,
    consumed: bool,
}

// Q: why can't we just wrap up these bounds into a trait?
// A: https://github.com/rust-lang/rust/issues/24616#issuecomment-112065997
// (The workaround mentioned in a later comment doesn't seem to be worth the added complexity)
impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    /// Construct a new `Multipart` with the given body reader and boundary.
    ///
    /// This will add the requisite `--` and CRLF (`\r\n`) to the boundary as per
    /// [IETF RFC 7578 section 4.1](https://tools.ietf.org/html/rfc7578#section-4.1).
    pub fn with_body<B: Into<String>>(stream: S, boundary: B) -> Self {
        let mut boundary = boundary.into();
        boundary.insert_str(0, "--");

        debug!("Boundary: {}", boundary);

        Multipart { 
            inner: BoundaryFinder::new(stream, boundary),
            read_hdr: ReadHeaders::default(),
            consumed: false,
        }
    }

    pub fn poll_field(&mut self) -> PollOpt<Field<S>, S::Error> {
        if !self.inner.consume_boundary()? {
            return ready(Ok(None));
        }

        if let Some(headers) = self.read_hdr.read_headers(&mut self.inner)? {
            ready(Ok(Some(field::new_field(headers, &mut self.inner))))
        } else {
            ready(Ok(None))
        }
    }

    pub fn fold_fields<T, F, Fut>(self, init: T, folder: F) -> FoldFields<F, T, S, Fut>
    where F: for<'a> FnMut(Field<'a, S>, T) -> Fut, Fut: Future<Item = Result<T, S::Error>> {
        FoldFields {
            folder,
            multipart: self,
            state: Some(init),
            fut: None,
        }
    }
}

pub struct FoldFields<F, T, S: Stream, Fut> {
    folder: F,
    multipart: Multipart<S>,
    state: Option<T>,
    fut: Option<Fut>,
}

impl<F, T, S, Fut> Future for FoldFields<F, T, S, Fut>
where F: for<'a> FnMut(Field<'a, S>, T) -> Fut, Fut: Future<Item = Result<T, S::Error>>,
S: Stream, S::Item: BodyChunk, S::Error: StreamError {
    type Output = Result<T, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        unsafe {
            self.map_unchecked_mut(|this| {
                loop {
                    if let Some(ref mut fut) = this.fut {
                        self.state = Some(Pin::new_unchecked(fut).poll()?);
                    }

                    if let Some(field) = this.multipart.poll_field()? {
                        let new_fut = (this.folder)(
                            this.state.expect(".poll() called after value returned"),
                            field
                        );
                    }
                }
            })
        }
    }
}

/// An extension trait for requests which may be multipart.
pub trait RequestExt: Sized {
    /// The success type, may contain `Multipart` or something else.
    type Multipart;

    /// Convert `Self` into `Self::Multipart` if applicable.
    fn into_multipart(self) -> Result<Self::Multipart, Self>;
}
