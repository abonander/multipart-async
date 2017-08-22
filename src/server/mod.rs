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

use mime::Mime;

use tempdir::TempDir;

use std::borrow::Borrow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
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

use self::field::ReadFields;

pub use self::field::{Field, FieldHeaders, FieldData};

pub use self::fold::FoldFields;

// FIXME: hyper integration once API is in place
// #[cfg(feature = "hyper")]
// mod hyper;

/// The server-side implementation of `multipart/form-data` requests.
///
pub struct Multipart<S: Stream> {
    stream: BoundaryFinder<S>,
    fields: field::ReadFields,
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
            stream: BoundaryFinder::new(stream, boundary),
            fields: ReadFields::new(),
        }
    }

    /// Poll for the current field in the form-data stream.
    ///
    /// This will return the same field until it is read to the end, or `next_field()` is called
    /// to skip the rest of the current field and begin reading the next one.
    ///
    /// To see how this method is meant to be used, see [the implementation of `FoldFields`](fold.html),
    /// which uses only `poll_field()` and `next_field()`.
    pub fn poll_field(&mut self) -> Poll<Option<Field<S>>, S::Error> {
        self.fields.poll_field(&mut self.stream)
    }

    /// Skip the rest of the current field, discarding any unread data from its body.
    ///
    /// This is also available as a method on `FieldData` (via `Field::data`) to help skirt issues
    /// with the borrow-checker.
    pub fn next_field(&mut self) {
        self.fields.next_field();
    }

    /// Get a future which runs a closure over all fields in the stream, collecting them to
    /// some state `T`.
    ///
    /// Note that this is not a typical `fold()` implementation which passes the fold state by-value
    /// and expects it to be returned (or the `futures::Stream` implementation which expects
    /// a future to be returned for each item). Instead, the closure may be invoked multiple times
    /// for the same field to completely extract its data, and is passed a mutable reference
    /// to the state.
    ///
    /// Return `Ok(Async::Ready(()))` to go to the next field.
    pub fn fold_fields<T, F>(self, init: T, folder: F) -> FoldFields<F, T, S> where F: FnMut(&mut T, Field<S>) -> Poll<(), S::Error> {
        FoldFields { folder, state: init, multipart: self }
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
