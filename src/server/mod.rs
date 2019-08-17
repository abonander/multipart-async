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

use futures::{Poll, Stream, Future};
use futures::task::{self, Context};

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

pub use self::field::{Field, FieldHeaders, FieldData};
// pub use self::field::{ReadTextField, TextField};

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
pub struct Multipart<S: TryStream> {
    inner: BoundaryFinder<S>,
    read_hdr: ReadHeaders,
    consumed: bool,
}

// Q: why can't we just wrap up these bounds into a trait?
// A: https://github.com/rust-lang/rust/issues/24616#issuecomment-112065997
// (The workaround mentioned in a later comment doesn't seem to be worth the added complexity)
impl<S> Multipart<S> where S: TryStream, S::Ok: BodyChunk, S::Error: StreamError {
    unsafe_pinned!(inner: BoundaryFinder<S>);
    unsafe_unpinned!(read_hdr: ReadHeaders);
    unsafe_unpinned!(consumed: bool);

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

    pub fn poll_field(mut self: Pin<&mut Self>, cx: &mut Context) -> PollOpt<Field<S>, S::Error> {
        if !ready!(self.as_mut().inner().consume_boundary(cx)?) {
            return Poll::Ready(None);
        }

        let maybe_headers = unsafe {
            let this = self.as_mut().get_unchecked_mut();
            ready!(this.read_hdr.read_headers(Pin::new_unchecked(&mut this.inner), cx)?)
        };

        if let Some(headers) = maybe_headers {
            ready_ok(field::new_field(headers, self.inner()))
        } else {
            Poll::Ready(None)
        }
    }

    pub fn fold_fields<F, Fut>(self, init: Fut::Ok, folder: F) -> FoldFields<F, S, Fut>
    where F: for<'a> FnMut(Field<'a, S>, Fut::Ok) -> Fut, Fut: TryFuture<Error = S::Error> {
        FoldFields {
            folder,
            multipart: self,
            state: Some(init),
            fut: None,
        }
    }
}

pub struct FoldFields<F, S: TryStream, Fut: TryFuture> {
    folder: F,
    multipart: Multipart<S>,
    state: Option<Fut::Ok>,
    fut: Option<Fut>,
}

impl<F, S: TryStream, Fut: TryFuture> FoldFields<F, S, Fut> {
    unsafe_pinned!(multipart: Multipart<S>);
    unsafe_pinned!(fut: Option<Fut>);
    unsafe_unpinned!(state: Option<Fut::Ok>);
    unsafe_unpinned!(folder: F);
}


impl<F, S, Fut> Future for FoldFields<F, S, Fut>
where F: for<'a> FnMut(Field<'a, S>, Fut::Ok) -> Fut, Fut: TryFuture<Error = S::Error>,
S: TryStream, S::Ok: BodyChunk, S::Error: StreamError {
    type Output = Result<Fut::Ok, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        loop {
            if let Some(fut) = self.as_mut().fut().as_pin_mut() {
                *self.as_mut().state() = Some(ready!(fut.try_poll(cx)?));
            }

            let state = self.as_mut().state()
                .take().expect(".poll() called after value returned");

            unsafe {
                let this = self.as_mut().get_unchecked_mut();
                if let Some(field) = ready!(Pin::new_unchecked(&mut this.multipart).poll_field(cx)?) {
                    let next_state = (this.folder)(field, state);
                    Pin::new_unchecked(&mut this.fut).set(Some(next_state));
                } else {
                    return ready_ok(state);
                }
            }

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
