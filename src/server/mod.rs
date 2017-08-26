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

use mime::Mime;

use tempdir::TempDir;

use std::borrow::Borrow;
use std::cell::Cell;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::Utf8Error;
use std::{fmt, io, mem, ptr};

use self::boundary::BoundaryFinder;

macro_rules! try_opt (
    ($expr:expr) => (
        match $expr {
            Some(val) => val,
            None => return None,
        }
    )
);

macro_rules! ret_err (
    ($string:expr) => (
        return ::helpers::error($string);
    );
    ($string:expr, $($args:tt)*) => (
        return ::helpers::error(format!($string, $($args)*));
    );
);

mod boundary;
mod field;

use helpers::*;

use self::field::ReadHeaders;

pub use self::field::{Field, FieldHeaders, FieldData};

// FIXME: hyper integration once API is in place
// #[cfg(feature = "hyper")]
// mod hyper;

/// The server-side implementation of `multipart/form-data` requests.
///
/// This will parse the incoming stream into `Field` which are returned by the
/// `Stream::poll()` implementation.
pub struct Multipart<S: Stream> {
    internal: Rc<Internal<S>>,
    read_hdr: ReadHeaders
}

// Q: why can't we just wrap up these bounds into a trait?
// A: https://github.com/rust-lang/rust/issues/24616#issuecomment-112065997
// (The workaround mentioned in a later comment doesn't seem to be worth the added complexity)
impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    /// Construct a new `Multipart` with the given body reader and boundary.
    /// This will prepend the requisite `"--"` to the boundary.
    pub fn with_body<B: Into<String>>(stream: S, boundary: B) -> Self {
        let mut boundary = boundary.into();
        boundary.insert_str(0, "--");

        debug!("Boundary: {}", boundary);

        Multipart { 
            internal: Rc::new(Internal::new(stream, boundary)),
            read_hdr: ReadHeaders::default(),
        }
    }
}

impl<S: Stream> Stream for Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = Field<S>;
    type Error = S::Error;

    /// Returns fields until the end of the stream.
    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        // FIXME: combine this with the next statement when non-lexical lifetimes are added
        // shouldn't be an issue anyway because the optimizer can fold these checks together
        if Rc::get_mut(&mut self.internal).is_none() {
            self.internal.park_curr_task();
            return not_ready();
        }

        // We don't want to return another `Field` unless we have exclusive access.
        let headers = {
            let stream = Rc::get_mut(&mut self.internal).unwrap().stream.get_mut();

            match try_ready!(self.read_hdr.read_headers(stream)) {
                Some(headers) => headers,
                None => return ready(None),
            }
        };

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

/// The operations required from a body stream's `Item` type.
pub trait BodyChunk: Sized {
    /// Split the chunk at `idx`, returning `(self[..idx], self[idx..])`.
    fn split_at(self, idx: usize) -> (Self, Self);

    /// Get the slice representing the data of this chunk.
    fn as_slice(&self) -> &[u8];

    /// Equivalent to `self.as_slice().len()`
    #[inline(always)]
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// Equivalent to `self.as_slice().is_empty()`
    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

    /// Equivalent to `self.as_slice().to_owned()`
    ///
    /// Implementors are welcome to override this if they can provide a cheaper conversion.
    #[inline(always)]
    fn into_vec(self) -> Vec<u8> {
        self.as_slice().to_owned()
    }
}

impl BodyChunk for Vec<u8> {
    fn split_at(mut self, idx: usize) -> (Self, Self) {
        let other = self.split_off(idx);
        (self, other)
    }

    fn as_slice(&self) -> &[u8] {
        self
    }

    fn into_vec(self) -> Vec<u8> { self }
}

impl<'a> BodyChunk for &'a [u8] {
    fn split_at(self, idx: usize) -> (Self, Self) {
        self.split_at(idx)
    }

    fn as_slice(&self) -> &[u8] {
        self
    }
}

/// The operations required from a body stream's `Error` type.
pub trait StreamError: From<io::Error> {
    /// Wrap a static string into this error type.
    ///
    /// Goes through `io::Error` by default.
    fn from_str(str: &'static str) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, str).into()
    }

    /// Wrap a dynamic string into this error type.
    ///
    /// Goes through `io::Error` by default.
    fn from_string(string: String) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, string).into()
    }

    /// Wrap a `std::str::Utf8Error` into this error type.
    ///
    /// Goes through `io::Error` by default.
    fn from_utf8(err: Utf8Error) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, err).into()
    }
}

impl<E> StreamError for E where E: From<io::Error> {}

//impl StreamError for String {
//    fn from_string(string: String) -> Self { string }
//}
