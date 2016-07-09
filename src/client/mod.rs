// Copyright 2016 `multipart` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! The client-side abstraction for multipart requests. Enabled with the `client` feature (on by
//! default).
//!
//! Use this when sending POST requests with files to a server.
use mime::Mime;
use buf_redux::{self, BufReader};

use std::borrow::Cow;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io;
use std::io::Cursor;
use std::io::prelude::*;

use std::path::Path;

#[cfg(feature = "hyper")]
pub mod hyper;

//pub mod lazy;

//mod sized;

//pub use self::sized::SizedRequest;

const BOUNDARY_LEN: usize = 16;

macro_rules! map_self {
    ($selff:expr, $try:expr) => (
        match $try {
            Ok(_) => Ok($selff),
            Err(err) => Err(err.into()),
        }
    )
}

/// The entry point of the client-side multipart API.
///
/// Though they perform I/O, the `.write_*()` methods do not return `io::Result<_>` in order to
/// facilitate method chaining. Upon the first error, all subsequent API calls will be no-ops until
/// `.send()` is called, at which point the error will be reported.
pub struct Multipart<B> {
    boundary: String,
    state: BodyState,
    body: Option<B>,
}

impl<B> Multipart<B> {
    
    /// Create a new instance with the given body.
    ///
    /// ##Note
    /// You might prefer to call `.build()` on the body for cleaner chaining.
    pub fn with_body(body: B) -> Self {
        Multipart {
            boundary: gen_boundary(),
            state: BodyState::NextField,
            body: Some(body),
        }
    }

    /// Create a new instance with the given body and buffer capacity.
    ///
    /// ##Note
    /// You might prefer to call `.with_buffer_cap()` on the body itself
    /// for brevity.
    pub fn with_buffer_cap(body: B, buffer_cap: usize) -> Self {
        Multipart {
            boundary: gen_boundary(),
            state: BodyState::NextField,
            body: Some(body),
        }
    }

    /// Create a new `Multipart` to wrap a request.
    ///
    /// ## Returns Error
    /// If `req.open_stream()` returns an error.
    pub fn on_request<R: Request>(&self, req: &mut R) {
        req.set_method();
        req.set_boundary(&self.boundary);
    }
}

impl<B: Body> Multipart<B> {
    fn on_writable<W: Write>(&mut self, out: &mut W) -> io::Result<RequestStatus> {
        use self::BodyState::*;
        match self.state {
            NextField => if self.body.is_some() {
                self.state = BodyState::BoundaryBefore;
                self.on_writable(out)
            } else {
                Ok(RequestStatus::NullRead)
            },
            BoundaryBefore => {

            }

        }
    }
}

#[derive(PartialEq, Eq, Copy, Clone)]
enum RequestStatus {
    MoreData,
    NullRead,
}

type FieldStatus = RequestStatus;

#[derive(PartialEq, Eq, Copy, Clone)]
enum BodyState {
    NextField,
    BoundaryBefore,
    FieldAttempted(bool),
    BoundaryAfter,
}

pub struct FieldHeader(Cursor<String>);

impl FieldHeader {
    fn text(name: &str) -> Self {
        Self::header(name, None, None)
    }

    fn file(name: &str, path: &Path) -> Self {
        let (mime, filename) = mime_filename(path);
        Self::header(name, filename, Some(&mime))
    }

    fn header(name: &str, filename: Option<&str>, content_type: Option<&Mime>) -> Self {
        let mut header = format!("Content-Disposition: form-data; name=\"{}\"", name);
        filename.map(|filename| write!(header, "; filename=\"{}\"", filename));
        content_type.map(|content_type| write!(header, "\r\nContent-Type: {}", content_type));
        header.push_str("\r\n\r\n");

        FieldHeader(Cursor::new(header))
    }
}

pub trait Field {
    fn write_out<W: Write>(&mut self, w: &mut W) -> io::Result<FieldStatus>;
}

pub struct TextField<'a> {
    header: FieldHeader,
    text: Cursor<CowStr<'a>>,
}

impl<'a> TextField<'a> {
    pub fn new<T: Into<Cow<'a, str>> + 'a>(name: &str, text: T) -> Self {
        TextField {
            header: FieldHeader::text(name),
            text: Cursor::new(CowStr::from(text)),
        }
    }
}

impl<'a> Field for TextField<'a> {
    fn write_out<W: Write>(&mut self, wrt: &mut W) -> io::Result<FieldStatus> {
        try!(buf_redux::copy_buf(&mut self.header.0, wrt));
        try!(buf_redux::copy_buf(&mut self.text, wrt));

        Ok(
            if cursor_at_end(&self.text) {
                RequestStatus::NullRead
            } else {
                RequestStatus::MoreData
            }
        )
    }
}

// Adapter to make Cow<'a, str> impl AsRef<[u8]>
struct CowStr<'a>(Cow<'a, str>);

impl<'a> AsRef<[u8]> for CowStr<'a> {
    fn as_ref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

impl<'a, S: Into<Cow<'a, str>>> From<S> for CowStr<'a> {
    fn from(cow: S) -> Self {
        CowStr(cow.into())
    }
}

pub struct StreamField<R> {
    header: FieldHeader,
    stream: R,
}

impl<R: BufRead> Field for StreamField<R> {
    fn write_out<W: Write>(&mut self, wrt: &mut W) -> io::Result<FieldStatus> {
        try!(buf_redux::copy_buf(&mut self.header.0, wrt));

        let read = try!(buf_redux::copy_buf(&mut self.stream, wrt));

        Ok(
            if cursor_at_end(&self.header.0) && read == 0 {
                RequestStatus::NullRead
            } else {
                RequestStatus::MoreData
            }
        )
    }
}
pub trait Request {
    fn set_method(&mut self);

    fn set_boundary(&mut self, boundary: &str);

    fn set_content_len(&mut self, content_len: u64);
}

pub trait Body {
    fn write_field<W: Write>(&mut self, wrt: &mut W) -> io::Result<FieldStatus>;
}

fn gen_boundary() -> String {
    ::random_alphanumeric(BOUNDARY_LEN)
}

fn mime_filename(path: &Path) -> (Mime, Option<&str>) {
    let content_type = ::mime_guess::guess_mime_type(path);
    let filename = opt_filename(path);
    (content_type, filename)
}

fn opt_filename(path: &Path) -> Option<&str> {
    path.file_name().and_then(|filename| filename.to_str())
}

fn cursor_at_end<T: AsRef<[u8]>>(cursor: &Cursor<T>) -> bool {
    let buf = cursor.get_ref().as_ref();
    let pos = cursor.position();
    pos == buf.len() as u64
}