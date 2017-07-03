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
pub struct Multipart<S> {
    reader: BoundaryFinder<S>,
}

impl<S> Multipart<S> {
    /// Construct a new `Multipart` with the given body reader and boundary.
    /// This will prepend the requisite `"--"` to the boundary.
    pub fn new<B: Into<String>>(stream: S, boundary: B) -> Self {
        let boundary = prepend_str("--", boundary.into());

        debug!("Boundary: {}", boundary);

        Multipart { 
            reader: BoundaryFinder::new(stream, boundary),
        }
    }


}

struct ContentType {
    val: Mime,

}

impl ContentType {
    fn read_from(line: &str) -> Option<ContentType> {
        const CONTENT_TYPE: &'static str = "Content-Type:";
        const BOUNDARY: &'static str = "boundary=\"";

        debug!("Reading Content-Type header from line: {:?}", line);

        if let Some((cont_type, after_cont_type)) = get_str_after(CONTENT_TYPE, ';', line) {
            let content_type = read_content_type(cont_type.trim());

            let boundary = get_str_after(BOUNDARY, '"', after_cont_type).map(|tup| tup.0.into());

            Some(ContentType {
                val: content_type,
            })
        } else {
            get_remainder_after(CONTENT_TYPE, line).map(|cont_type| {
                let content_type = read_content_type(cont_type.trim());
                ContentType { val: content_type }
            })
        }
    }
}

fn read_content_type(cont_type: &str) -> Mime {
    cont_type.parse().ok().unwrap_or_else(::mime_guess::octet_stream)
}

#[derive(Default)]
struct ContentDisp {
    field_name: String,
    filename: Option<String>,
}

impl ContentDisp {
    fn read_from(line: &str) -> Option<ContentDisp> {
        debug!("Reading Content-Disposition from line: {:?}", line);

        if line.is_empty() {
            return None;
        }

        const CONT_DISP: &'static str = "Content-Disposition:";
        const NAME: &'static str = "name=\"";
        const FILENAME: &'static str = "filename=\"";

        let after_disp_type = {
            let (disp_type, after_disp_type) = try_opt!(get_str_after(CONT_DISP, ';', line));
            let disp_type = disp_type.trim();

            if disp_type != "form-data" {
                error!("Unexpected Content-Disposition value: {:?}", disp_type);
                return None;
            }

            after_disp_type
        };

        let (field_name, after_field_name) = try_opt!(get_str_after(NAME, '"', after_disp_type));

        let filename = get_str_after(FILENAME, '"', after_field_name)
            .map(|(filename, _)| filename.to_owned());

        Some(ContentDisp { field_name: field_name.to_owned(), filename: filename })
    }
}

/// Get the string after `needle` in `haystack`, stopping before `end_val_delim`
fn get_str_after<'a>(needle: &str, end_val_delim: char, haystack: &'a str) -> Option<(&'a str, &'a str)> {
    let val_start_idx = try_opt!(haystack.find(needle)) + needle.len();
    let val_end_idx = try_opt!(haystack[val_start_idx..].find(end_val_delim)) + val_start_idx;
    Some((&haystack[val_start_idx..val_end_idx], &haystack[val_end_idx..]))
}

/// Get everything after `needle` in `haystack`
fn get_remainder_after<'a>(needle: &str, haystack: &'a str) -> Option<(&'a str)> {
    let val_start_idx = try_opt!(haystack.find(needle)) + needle.len();
    Some(&haystack[val_start_idx..])
}

fn valid_subslice(bytes: &[u8]) -> &str {
    ::std::str::from_utf8(bytes)
        .unwrap_or_else(|err| unsafe { ::std::str::from_utf8_unchecked(&bytes[..err.valid_up_to()]) })
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

pub trait BodyStream: Stream where Self::Item: BodyChunk, Self::Error: From<io::Error> {}

impl<T> BodyStream for T where T: Stream, T::Item: BodyChunk, T::Error: From<io::Error> {}
