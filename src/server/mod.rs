// Copyright 2016 `multipart` Crate Developers
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

use mime::Mime;

use tempdir::TempDir;

use std::borrow::Borrow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::{fmt, io, mem, ptr};

use self::boundary::BoundaryFinder;
use self::Error::*;

macro_rules! try_opt (
    ($expr:expr) => (
        match $expr {
            Some(val) => val,
            None => return None,
        }
    )
);

macro_rules! try_opt_readmore (
    ($expr:expr) => (
        match $expr {
            Some(val) => val,
            None => return Err(ReadMore),
        }
    )
);

mod boundary;

#[cfg(feature = "hyper")]
mod hyper;

#[cfg(feature = "hyper")]
pub use RequestHandler;

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
    pub fn new<B: Into<String>>(stream:S, boundary: B) -> Self {
        let boundary = prepend_str("--", boundary.into());

        debug!("Boundary: {}", boundary);

        Multipart { 
            reader: BoundaryFinder::new(s, boundary),
        }
    }

    pub fn get_field(&mut self) -> Option<(&Field, FieldData)> {
        if let State::Field(ref field) = self.state { 
            Some((field, FieldData::new(field, &mut self.reader)))
        } else {
            None
        }
    }

    fn next_state(&mut self) -> Result<()> {
        self.state = match self.state {
            State::ContDisp => {
                let mut buf = String::new();
                let _ = try!(self.reader.try_read_line(&mut buf));
                
                let cont_disp = try_opt_readmore!(ContentDisp::read_from(&buf));
                
                State::ContType(cont_disp)
            },
            State::ContType(ref mut cont_disp) => {
                let mut buf = String::new();
                let _ = try!(self.reader.try_read_line(&mut buf));

                let cont_disp = mem::replace(cont_disp, ContentDisp::default()); 
                let cont_type = ContentType::read_from(&buf);

                State::Field(Field::new(cont_disp, cont_type))
            },
            State::Field(_) => State::ContDisp,
            State::AtEnd => State::AtEnd,
        };

        Ok(())
    } 

    fn consume_boundary(&mut self) -> Result<bool> {
        try!(self.reader.consume_boundary());

        let mut out = [0; 2];
        let _ = self.reader.read(&mut out);

        if *b"\r\n" == out {
            Ok(true)
        } else {
            if *b"--" != out {
                warn!("Unexpected 2-bytes after boundary: {:?}", out);
            }

            self.state = State::AtEnd;

            Ok(false)
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

pub struct Field {
    name: String,
    cont_type: Option<Mime>,
    filename: Option<String>,
    #[allow(dead_code)]
    boundary: Option<String>,
}

impl Field {
    fn new(cont_disp: ContentDisp, cont_type: Option<ContentType>) -> Self {
        let (cont_type, boundary) = match cont_type {
            Some(ct) => (Some(ct.val), ct.boundary),
            None => (None, None),
        };

        Field {
            name: cont_disp.field_name,
            cont_type: cont_type,
            filename: cont_disp.filename,
            boundary: boundary
        }
    }
    
    pub fn is_text(&self) -> bool {
        self.cont_type.is_none()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn content_type(&self) -> Option<&Mime> {
        self.cont_type.as_ref()
    }
}

pub struct FieldData<'a> {
    field: &'a Field,
    src: &'a mut BoundaryFinder,
}

impl<'a> FieldData<'a> {
    fn new(field: &'a Field, src: &'a mut BoundaryFinder) -> Self {
        FieldData {
            field: field,
            src: src,
        }
    }

    pub fn available(&self) -> usize {
        self.src.available()    
    }

    pub fn read_string(&mut self) -> Result<Option<&str>> {
        if !self.field.is_text() { return Ok(None); }
        
        let _ = self.src.find_boundary();

        if !self.src.boundary_read() { return Err(ReadMore); }

        let buf = self.src.find_boundary();

        Ok(Some(::std::str::from_utf8(buf).unwrap()))
    }

    pub fn read_string_partial(&mut self) -> Option<&str> {
        if !self.field.is_text() { return None; }

        let buf = self.src.find_boundary();

        Some(valid_subslice(buf))
    }

    pub fn read_adapter(&mut self) -> Option<ReadField> {
        if self.field.is_text() { return None; }

        Some(ReadField { src: &mut self.src })
    }
}

pub struct ReadField<'a> {
    src: &'a mut BoundaryFinder,
}

impl<'a> Read for ReadField<'a> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.src.read(out)
    }
}

impl<'a> BufRead for ReadField<'a> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.src.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.src.consume(amt)
    }
}

/// A result of `Multipart::save_all()`.
#[derive(Debug)]
pub struct Entries {
    /// The text fields of the multipart request, mapped by field name -> value.
    pub fields: HashMap<String, String>,
    /// A map of file field names to their contents saved on the filesystem.
    pub files: HashMap<String, SavedFile>,
    /// The directory the files in this request were saved under; may be temporary or permanent.
    pub dir: SaveDir,
}

impl Entries {
    fn new_tempdir_in<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        TempDir::new_in(path, "multipart").map(Self::with_tempdir)
    }

    fn new_tempdir() -> io::Result<Self> {
        TempDir::new("multipart").map(Self::with_tempdir)
    }

    fn with_tempdir(tempdir: TempDir) -> Entries {
        Entries {
            fields: HashMap::new(),
            files: HashMap::new(),
            dir: SaveDir::Temp(tempdir),
        }
    }
}



/// A file saved to the local filesystem from a multipart request.
#[derive(Debug)]
pub struct SavedFile {
    /// The complete path this file was saved at.
    pub path: PathBuf,

    /// The original filename of this file, if one was provided in the request.
    ///
    /// ##Warning
    /// You should treat this value as untrustworthy because it is an arbitrary string provided by
    /// the client. You should *not* blindly append it to a directory path and save the file there, 
    /// as such behavior could easily be exploited by a malicious client.
    pub filename: Option<String>,

    /// The number of bytes written to the disk; may be truncated.
    pub size: u64,
}

/// The result of [`Multipart::save_all()`](struct.multipart.html#method.save_all).
#[derive(Debug)]
pub enum SaveResult {
    /// The operation was a total success. Contained are all entries of the request.
    Full(Entries),
    /// The operation errored partway through. Contained are the entries gathered thus far,
    /// as well as the error that ended the process.
    Partial(Entries, io::Error),
    /// The `TempDir` for `Entries` could not be constructed. Contained is the error detailing the
    /// problem.
    Error(io::Error),
}

impl SaveResult {
    /// Take the `Entries` from `self`, if applicable, and discarding
    /// the error, if any.
    pub fn to_entries(self) -> Option<Entries> {
        use self::SaveResult::*;

        match self {
            Full(entries) | Partial(entries, _) => Some(entries),
            Error(_) => None,
        }
    }

    /// Decompose `self` to `(Option<Entries>, Option<io::Error>)`
    pub fn to_opt(self) -> (Option<Entries>, Option<io::Error>) {
        use self::SaveResult::*;

        match self {
            Full(entries) => (Some(entries), None),
            Partial(entries, error) => (Some(entries), Some(error)),
            Error(error) => (None, Some(error)),
        }
    }

    /// Map `self` to an `io::Result`, discarding the error in the `Partial` case.
    pub fn to_result(self) -> io::Result<Entries> {
        use self::SaveResult::*;

        match self {
            Full(entries) | Partial(entries, _) => Ok(entries),
            Error(error) => Err(error),
        }
    }
}

fn retry_on_interrupt<F, T>(mut do_fn: F) -> io::Result<T> where F: FnMut() -> io::Result<T> {
    loop {
        match do_fn() {
            Ok(val) => return Ok(val),
            Err(err) => if err.kind() != io::ErrorKind::Interrupted { 
                return Err(err);
            },
        }
    }
} 

fn prepend_str(prefix: &str, mut string: String) -> String {
    string.reserve(prefix.len());

    unsafe {
        let bytes = string.as_mut_vec();

        // This addition is safe because it was already done in `String::reserve()`
        // which would have panicked if it overflowed.
        let old_len = bytes.len();
        let new_len = bytes.len() + prefix.len();
        bytes.set_len(new_len);

        ptr::copy(bytes.as_ptr(), bytes[prefix.len()..].as_mut_ptr(), old_len);
        ptr::copy(prefix.as_ptr(), bytes.as_mut_ptr(), prefix.len());
    }

    string
}

fn create_full_path(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        try!(fs::create_dir_all(parent));
    } else {
        // RFC: return an error instead?
        warn!("Attempting to save file in what looks like a root directory. File path: {:?}", path);
    }

    File::create(&path)
}

/// A wrapper around `Multipart` which conveniently handles body parsing and provides all fields
/// in one convenient representation.
pub struct EasyHandler<H> {
    multi: Option<Multipart>,
    entries: Entries,
    handler: H
}

impl<H> EasyHandler<H> {
    pub fn new(handler: H) -> io::Result<Self> {
        let entries = try!(Entries::new_tempdir());

        EasyHandler {
            multi: None,
            entries: entries,
            handler: handler
        }
    }

    pub fn new_tempdir_in<P: AsRef<Path>>(handler: H, path: P) -> io::Result<Self> {
        let entries = try!(Entries::new_tempdir());

        EasyHandler {
            multi: None,
            entries: entries,
            handler: handler
        }
    }

    pub fn new_permdir<P: Into<PathBuf>>(handler: H, permdir: P) -> io::Result<Self> {
        let entries = try!(Entries::new_permdir(permdir));

        EasyHandler {
            multi: None,
            entries: entries,
            handler: handler,
        }
    }

    fn set_boundary<B: Into<String>>(&mut self, boundary: B) {
        self.multi = Some(Multipart::new(boundary));
    }
}

pub trait RequestHandler {
    type Response: Write;

    fn on_request(entries: &Entries, response: &mut Self::Response) -> bool;
}


pub trait BodyChunk {
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
