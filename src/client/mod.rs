// Copyright 2017 `multipart-async` Crate Developers
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
use buf_redux::{self, BufReader, copy_buf};

use std::borrow::Cow;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io;
use std::io::Cursor;
use std::io::prelude::*;

use std::path::Path;

use self::RequestStatus::*;

#[cfg(feature = "hyper")]
pub mod hyper;

#[macro_export]
macro_rules! field (
    ($name:expr => text($val:expr)) => (
        $crate::client::TextField::new($name, $val)
    );
    ($name:expr => file($val:expr)) => (
        $crate::client::StreamField::open_file($name, $val)
            .expect("Could not open file for reading")
    );
    ($name:expr => stream($val:expr)) => (
        $crate::client::StreamField::buffer(
            $name, $val, None, None
        )
    );
    ($name:expr => stream($val:expr, $content_type:expr)) => (
        $crate::client::StreamField::buffer(
            $name, $val, Some(&$content_type), None
        )
    );
    ($name:expr => buffered($val:expr)) => (
        $crate::client::StreamField::new(
            $name, $val, None, None
        )
    );
    ($name:expr => buffered($val:expr, $cont_type:expr)) => (
        $crate::client::StreamField::new(
            $name, $val, Some(&$cont_type), None
        )
    );
);

pub mod chained;

pub mod dyn;

//pub mod lazy;

//mod sized;

//pub use self::sized::SizedRequest;

const BOUNDARY_LEN: usize = 16;


macro_rules! wrap_const {
    ($($name:ident = $val:expr),+,) => (
        $(wrap_const! { $name = $val })+
    );
    ($name:ident = $val:expr) => (
        struct $name;

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                $val.as_bytes()
            }
        }
    )
}

/// The entry point of the client-side multipart API.
///
/// Though they perform I/O, the `.write_*()` methods do not return `io::Result<_>` in order to
/// facilitate method chaining. Upon the first error, all subsequent API calls will be no-ops until
/// `.send()` is called, at which point the error will be reported.
pub struct Multipart<B> {
    boundary: Boundary,
    end: Cursor<End>,
    state: BodyState,
    body: B,
}

impl<B> Multipart<B> {
    
    /// Create a new instance with the given body.
    ///
    /// ##Note
    /// You might prefer to call `.build()` on the body for cleaner chaining.
    pub fn with_body(body: B) -> Self {
        let mut boundary = Boundary::gen();

        boundary.skip_pre_newline();
    
        Multipart {
            boundary: boundary,
            end: Cursor::new(End),
            state: BodyState::WriteBoundary,
            body: body,
        }
    }

    /// Create a new `Multipart` to wrap a request.
    ///
    /// ## Returns Error
    /// If `req.open_stream()` returns an error.
    pub fn on_request<R: Request>(&self, req: &mut R) {
        req.set_method();
        req.set_boundary(self.boundary.get());
    }
}

impl<B: Body> Multipart<B> {
    pub fn on_writable<W: Write>(&mut self, out: &mut W) -> io::Result<RequestStatus> {
        use self::BodyState::*;

        loop {
            match self.state {
                NextField => if !self.body.finished() {
                    self.state.go_next();
                } else if try!(self.end.try_write_all(out)) {
                    return Ok(NullRead);
                } else {
                    return Ok(MoreData);
                },
                WriteBoundary => {
                    if let MoreData = try!(self.boundary.write_to(out)) {
                        debug!("More data!");
                        return Ok(MoreData);
                    }
                    
                    if self.boundary.written() {
                        self.boundary.reset();
                        self.state.go_next();
                    }
                },
                WriteField => if let NullRead = try!(self.body.write_field(out)) {
                    self.state.go_next();
                    return Ok(MoreData);
                },
                FieldNulled => if let NullRead = try!(self.body.write_field(out)) {
                    self.body.pop_field();
                    self.state.go_next();
                } else {
                    self.state = WriteField;
                },
            }
        }
    }
}

impl<B: Body> From<B> for Multipart<B> {
    fn from(body: B) -> Self {
        Self::with_body(body)
    }
}

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum RequestStatus {
    MoreData,
    NullRead,
}

pub type FieldStatus = RequestStatus;

#[derive(PartialEq, Eq, Copy, Clone)]
enum BodyState {
    NextField,
    WriteField,
    FieldNulled,
    WriteBoundary,
}

impl BodyState {
    fn go_next(&mut self) {
        use self::BodyState::*;

        *self = match *self {
            NextField => WriteField,
            WriteField => FieldNulled,
            FieldNulled => WriteBoundary,
            WriteBoundary => NextField,
        }
    }
}

pub struct FieldHeader(Cursor<String>);

impl FieldHeader {
    fn text(name: &str) -> Self {
        Self::new(name, None, None)
    }

    fn file(name: &str, path: &Path) -> Self {
        let (mime, filename) = mime_filename(path);
        Self::new(name, Some(&mime), filename)
    }

    fn stream(name: &str, content_type: Option<&Mime>, filename: Option<&str>) -> Self {
        if let Some(content_type) = content_type {
            Self::new(name, Some(&content_type), filename)
        } else {
            Self::new(name, Some(&::mime_guess::octet_stream()), None)
        }
    }

    fn new(name: &str, content_type: Option<&Mime>, filename: Option<&str>) -> Self {
        let mut header = format!("\r\nContent-Disposition: form-data; name=\"{}\"", name);
        filename.map(|filename| write!(header, "; filename=\"{}\"", filename));
        content_type.map(|content_type| write!(header, "\r\nContent-Type: {}", content_type));
        header.push_str("\r\n\r\n");

        FieldHeader(Cursor::new(header))
    }

    fn at_end(&self) -> bool { self.0.at_end() }
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
            if self.text.at_end() {
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

pub type FileField = StreamField<BufReader<File>>;

pub struct StreamField<R> {
    header: FieldHeader,
    stream: R,
}

impl<R: Read> StreamField<BufReader<R>> {
    pub fn new(name: &str, stream: R, content_type: Option<&Mime>, filename: Option<&str>) -> Self {
        StreamField::buffered(name, BufReader::new(stream), content_type, filename)
    }

    pub fn boxed<'a>(self) -> StreamField<BufReader<Box<Read + 'a>>> where R: 'a {
        StreamField {
            header: self.header,
            stream: self.stream.boxed(),
        }
    }
}

impl<R: BufRead> StreamField<R> {
    pub fn buffered(name: &str, stream: R, content_type: Option<&Mime>, filename: Option<&str>) -> Self {
        StreamField {
            header: FieldHeader::stream(name, content_type, filename),
            stream: stream,
        }
    }

    pub fn boxed_buffered<'a>(self) -> StreamField<Box<BufRead + 'a>> where R: 'a {
        let stream: Box<BufRead + 'a> = Box::new(self.stream);
        
        StreamField {
            header: self.header,
            stream: stream,
        }
    }
}

impl FileField {
    pub fn open_file<P: AsRef<Path>>(name: &str, path: P) -> io::Result<Self> {
        let (mime, filename) = mime_filename(path.as_ref());
        let file = try!(File::open(path.as_ref()));

        Ok(Self::new(name, file, Some(&mime), filename))
    }
}

impl<R: BufRead> Field for StreamField<R> {
    fn write_out<W: Write>(&mut self, wrt: &mut W) -> io::Result<FieldStatus> {
        try!(buf_redux::copy_buf(&mut self.header.0, wrt));

        let read = try!(buf_redux::copy_buf(&mut self.stream, wrt));

        Ok(
            if self.header.at_end() && read == 0 {
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

    fn pop_field(&mut self);

    fn finished(&self) -> bool;

    fn build(self) -> Multipart<Self> where Self: Sized {
        Multipart::with_body(self)
    }
}

impl<'a, B: Body> Body for &'a mut B {
    fn write_field<W: Write>(&mut self, wrt: &mut W) -> io::Result<FieldStatus> {
        (**self).write_field(wrt)
    }

    fn pop_field(&mut self) {
        (**self).pop_field();
    }

    fn finished(&self) -> bool {
        (**self).finished()
    }
}

wrap_const! {
    Prefix = "\r\n--",
    End = "--",
}

struct Boundary {
    prefix: Cursor<Prefix>,
    boundary: Cursor<String>,
}

impl Boundary {
    fn gen() -> Self {
        let boundary = ::random_alphanumeric(BOUNDARY_LEN);

        Boundary {
            prefix: Cursor::new(Prefix),
            boundary: Cursor::new(boundary),
        }
    }

    fn write_to<W: Write>(&mut self, w: &mut W) -> io::Result<RequestStatus> {
        if !try!(self.prefix.try_write_all(w)) {
            return Ok(MoreData);
        }

        if !try!(self.boundary.try_write_all(w)) {
            return Ok(MoreData);
        }

        Ok(NullRead)
    }

    fn written(&self) -> bool {
        self.prefix.at_end() &&
            self.boundary.at_end()
    }

    fn skip_pre_newline(&mut self) {
        self.prefix.set_position(2);
    }

    fn reset(&mut self) {
        self.prefix.reset();
        self.boundary.reset();
    }

    fn get(&self) -> &str {
        self.boundary.get_ref()
    }
}

fn mime_filename(path: &Path) -> (Mime, Option<&str>) {
    let content_type = ::mime_guess::guess_mime_type(path);
    let filename = opt_filename(path);
    (content_type, filename)
}

fn opt_filename(path: &Path) -> Option<&str> {
    path.file_name().and_then(|filename| filename.to_str())
}

trait CursorExt {
    fn at_end(&self) -> bool;

    fn reset(&mut self);

    fn try_write_all<W: Write>(&mut self, w: &mut W) -> io::Result<bool>;
}

impl<T: AsRef<[u8]>> CursorExt for Cursor<T> {
    fn at_end(&self) -> bool {
        let buf = self.get_ref().as_ref();
        let pos = self.position();
        pos == buf.len() as u64
    }

    fn reset(&mut self) {
        self.set_position(0);
    }

    fn try_write_all<W: Write>(&mut self, w: &mut W) -> io::Result<bool> {
        try!(copy_buf(self, w));

        Ok(self.at_end())
    }
}
