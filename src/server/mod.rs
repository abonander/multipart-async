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
use std::cell::{Cell, RefCell, RefMut};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::string::FromUtf8Error;
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


mod boundary;
mod field;

mod fold;

use helpers::*;

use self::field::ReadHeaders;

pub use self::field::{Field, FieldHeaders, FieldData};

pub use self::fold::FoldFields;

// FIXME: hyper integration once API is in place
// #[cfg(feature = "hyper")]
// mod hyper;

/// The server-side implementation of `multipart/form-data` requests.
pub struct Multipart<S: Stream> {
    internal: Rc<Internal<S>>,
    read_hdr: ReadHeaders
}

// Q: why can't we just wrap up these bounds into a trait?
// A: https://github.com/rust-lang/rust/issues/24616#issuecomment-112065997
// (The workaround mentioned below doesn't seem to be worth the added complexity)
impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    /// Construct a new `Multipart` with the given body reader and boundary.
    /// This will prepend the requisite `"--"` to the boundary.
    pub fn with_body<B: Into<String>>(stream: S, boundary: B) -> Self {
        let mut boundary = boundary.into();
        boundary.insert_str(0, "--");

        debug!("Boundary: {}", boundary);

        Multipart { 
            internal: Rc::new(Internal::new(stream, boundary)),
            read_hdr: ReadHeaders,
        }
    }

    pub fn on_field(&self) -> bool {
        self.internal.on_field.get()
    }
}

impl<S: Stream> Stream for Multipart<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = Field<S>;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.internal.on_field.get() { return not_ready(); }

        unimplemented!()
    }
}

struct Internal<S: Stream> {
    stream: RefCell<BoundaryFinder<S>>,
    on_field: Cell<bool>,
    waiting_task: Cell<Option<Task>>,
}

impl<S: Stream> Internal<S> where S::Item: BodyChunk, S::Error: StreamError {
    fn new(stream: S, boundary: String) -> Self {
        debug_assert!(boundary.starts_with("--"), "Boundary must start with --");

        Internal {
            stream: BoundaryFinder::new(stream, boundary).into(),
            on_field: false.into(),
            waiting_task: None,
        }
    }

    fn park_curr_task(&self) {
        self.waiting_task.set(Some(task::current()));
    }

    fn notify_task(&self) {
        self.waiting_task.take().map(|t| t.notify());
    }

    fn stream_mut(&self) -> RefMut<BoundaryFinder<S>> {
        self.stream.borrow_mut()
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
pub trait StreamError: From<io::Error> + From<FromUtf8Error> {
    fn from_string(string: String) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, string).into()
    }
}

impl<E> StreamError for E where E: From<io::Error> + From<FromUtf8Error> {}

//impl StreamError for String {
//    fn from_string(string: String) -> Self { string }
//}
