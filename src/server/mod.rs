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

use futures::Stream;

use mime::Mime;

use tempdir::TempDir;

use std::borrow::Borrow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
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

// FIXME: hyper integration once API is in place
// #[cfg(feature = "hyper")]
// mod hyper;


const RANDOM_FILENAME_LEN: usize = 12;

/// The server-side implementation of `multipart/form-data` requests.
///
/// Implements `Borrow<R>` to allow access to the request body, if desired.
pub struct Multipart<S: Stream> {
    reader: BoundaryFinder<S>,
}

impl<S: Stream> Multipart<S> where S::Item: BodyChunk, S::Error: From<io::Error> {
    /// Construct a new `Multipart` with the given body reader and boundary.
    /// This will prepend the requisite `"--"` to the boundary.
    pub fn new<B: Into<String>>(stream: S, boundary: B) -> Self {
        let mut boundary = boundary.into();
        boundary.insert_str(0, "--");

        debug!("Boundary: {}", boundary);

        Multipart { 
            reader: BoundaryFinder::new(stream, boundary),
        }
    }
}

pub trait BodyChunk: Sized {
    fn split_at(self, idx: usize) -> (Self, Self);

    fn as_slice(&self) -> &[u8];

    fn from_vec(vec: Vec<u8>) -> Self;

    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn is_empty(&self) -> bool {
        self.as_slice().is_empty()
    }

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

    fn from_vec(vec: Vec<u8>) -> Self {
        vec
    }
}
