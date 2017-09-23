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
use futures::task::{self, Task};

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
    internal: Rc<Internal<S>>,
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
            internal: Rc::new(Internal::new(stream, boundary)),
            read_hdr: ReadHeaders::default(),
            consumed: false,
        }
    }
}

impl<S: Stream> Stream for Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = Field<S>;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        // FIXME: combine this with the next statement when non-lexical lifetimes are added
        // shouldn't be an issue anyway because the optimizer can fold these checks together
        if Rc::get_mut(&mut self.internal).is_none() {
            debug!("returning NotReady, field was in flight");
            self.internal.park_curr_task();
            return not_ready();
        }

        // We don't want to return another `Field` unless we have exclusive access.
        let headers = {
            let stream = Rc::get_mut(&mut self.internal).unwrap().stream.get_mut();

            // only attempt to consume the boundary if it hasn't been done yet
            self.consumed = self.consumed || try_ready!(stream.consume_boundary());

            if !self.consumed {
                return ready(None);
            }

            match try_ready!(self.read_hdr.read_headers(stream)) {
                Some(headers) => headers,
                None => return ready(None),
            }
        };

        // the boundary should be consumed the next time poll() is ready to move forward
        self.consumed = false;

        info!("read field: {:?}", headers);

        ready(field::new_field(headers, self.internal.clone()))
    }
}

struct Internal<S: Stream> {
    stream: Cell<BoundaryFinder<S>>,
    waiting_task: Cell<Option<Task>>,
}

impl<S: Stream> Internal<S> {
    fn new(stream: S, boundary: String) -> Self {
        debug_assert!(boundary.starts_with("--"), "Boundary must start with --");

        Internal {
            stream: BoundaryFinder::new(stream, boundary).into(),
            waiting_task: None.into(),
        }
    }

    fn park_curr_task(&self) {
        self.waiting_task.set(Some(task::current()));
    }

    fn notify_task(&self) {
        self.waiting_task.take().map(|t| t.notify());
    }
}

/// An extension trait for requests which may be multipart.
pub trait RequestExt: Sized {
    /// The success type, may contain `Multipart` or something else.
    type Multipart;

    /// Convert `Self` into `Self::Multipart` if applicable.
    fn into_multipart(self) -> Result<Self::Multipart, Self>;
}
