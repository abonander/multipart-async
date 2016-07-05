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

use std::borrow::Cow;
use std::fs::File;
use std::io;
use std::io::prelude::*;

use std::path::Path;

#[cfg(feature = "hyper")]
pub mod hyper;

pub mod lazy;

mod sized;

pub use self::sized::SizedRequest;

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
    buffer: Buffer,
    body: B,
}

impl<B> Multipart<B> {
    
    /// Create a new instance with the given body.
    ///
    /// ##Note
    /// You might prefer to call `.build()` on the body for cleaner chaining.
    pub fn with_body(body: B) -> Self {
        Multipart {
            boundary: gen_boundary(),
            buffer: Buffer::new(),
            body: body,
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
            buffer: Buffer::with_capacity(buffer_cap),
            body: body,
        }
    }

    /// Create a new `Multipart` to wrap a request.
    ///
    /// ## Returns Error
    /// If `req.open_stream()` returns an error.
    pub fn on_request<R: Request>(&self, req: &mut R) {
        req.set_method_post();
        req.set_boundary(&self.boundary);
    }

}

impl<B: Body> Multipart<B> {
    /// Signal to the handler that it has time to read into the buffer.
    fn fill_buf(&mut self) -> io::Result<RequestStatus> {
        if !self.buffer.is_empty() {
            return Ok(RequestStatus::Done);
        }

        let mut field = self.body.current_field();

        
    }

    fn on_writable<W: Write>(&mut self, out: &mut W) -> io::Result<RequestStatus> {
        if let RequestStatus::MoreData == try!(self.fill_buf()) {
            try!(self.buffer.write_to(out));
            Ok(RequestFields::MoreData)
        } else {
            Ok(RequestFields::Done)
        }
    }
}


#[derive(PartialEq, Eq, Copy, Clone)]
enum RequestStatus {
    MoreData,
    Done,
}

impl<B> Multipart<B> { 
    /// Write a text field to this multipart request.
    /// `name` and `val` can be either owned `String` or `&str`.
    ///
    /// ##Errors
    /// If something went wrong with the HTTP stream.
    pub fn write_text<N: AsRef<str>, V: AsRef<str>>(&mut self, name: N, val: V) -> Result<&mut Self, S::Error> {
        map_self!(self, self.writer.write_text(name.as_ref(), val.as_ref()))
    }
    
    /// Open a file pointed to by `path` and write its contents to the multipart request, 
    /// supplying its filename and guessing its `Content-Type` from its extension.
    ///
    /// If you want to set these values manually, or use another type that implements `Read`, 
    /// use `.write_stream()`.
    ///
    /// `name` can be either `String` or `&str`, and `path` can be `PathBuf or `&Path`.
    ///
    /// ##Errors
    /// If there was a problem opening the file (was a directory or didn't exist),
    /// or if something went wrong with the HTTP stream.
    pub fn write_file<N: AsRef<str>, P: AsRef<Path>>(&mut self, name: N, path: P) -> Result<&mut Self, S::Error> {
        let name = name.as_ref();
        let path = path.as_ref();

        map_self!(self, self.writer.write_file(name, path))
    }

    /// Write a byte stream to the multipart request as a file field, supplying `filename` if given,
    /// and `content_type` if given or `"application/octet-stream"` if not.
    ///
    /// `name` can be either `String` or `&str`, and `read` can take the `Read` by-value or
    /// with an `&mut` borrow.
    ///
    /// ##Warning
    /// The given `Read` **must** be able to read to EOF (end of file/no more data), meaning
    /// `Read::read()` returns `Ok(0)`. If it never returns EOF it will be read to infinity 
    /// and the request will never be completed.
    ///
    /// When using `SizedRequest` this also can cause out-of-control memory usage as the
    /// multipart data has to be written to an in-memory buffer so its size can be calculated.
    ///
    /// Use `Read::take()` if you wish to send data from a `Read` 
    /// that will never return EOF otherwise.
    ///
    /// ##Errors
    /// If the reader returned an error, or if something went wrong with the HTTP stream.
    // RFC: How to format this declaration?
    pub fn write_stream<N: AsRef<str>, St: Read>(
        &mut self, name: N, stream: &mut St, filename: Option<&str>, content_type: Option<Mime>
    ) -> Result<&mut Self, S::Error> {
        let name = name.as_ref();

        map_self!(self, self.writer.write_stream(stream, name, filename, content_type))
    } 

    /// Finalize the request and return the response from the server, or the last error if set.
    pub fn send(self) -> Result<S::Response, S::Error> {
        self.writer.finish().map_err(io::Error::into).and_then(|body| body.finish())
    }    
}

impl<R: HttpRequest> Multipart<SizedRequest<R>>
where <R::Stream as HttpStream>::Error: From<R::Error> {
    /// Create a new `Multipart` using the `SizedRequest` wrapper around `req`.
    pub fn from_request_sized(req: R) -> Result<Self, R::Error> {
        Multipart::from_request(SizedRequest::from_request(req))
    }
}


pub trait Request {
    fn set_method_post(&mut self);

    fn set_boundary(&mut self, boundary: &str);
}

fn gen_boundary() -> String {
    ::random_alphanumeric(BOUNDARY_LEN)
}

fn open_stream<R: HttpRequest>(mut req: R, content_len: Option<u64>) -> Result<(String, R::Stream), R::Error> {
    let boundary = gen_boundary();
    req.apply_headers(&boundary, content_len);
    req.open_stream().map(|stream| (boundary, stream))
}

struct MultipartWriter<'a, W> {
    inner: W,
    boundary: Cow<'a, str>,
    data_written: bool,
}

impl<'a, W: Write> MultipartWriter<'a, W> {
    fn new<B: Into<Cow<'a, str>>>(inner: W, boundary: B) -> Self {
        MultipartWriter {
            inner: inner,
            boundary: boundary.into(),
            data_written: false,
        }
    }

    fn write_boundary(&mut self) -> io::Result<()> {
        write!(self.inner, "\r\n--{}\r\n", self.boundary)
    }

    fn write_text(&mut self, name: &str, text: &str) -> io::Result<()> {
        chain_result! {
            self.write_field_headers(name, None, None),
            self.inner.write_all(text.as_bytes())
        }
    }

    fn write_file(&mut self, name: &str, path: &Path) -> io::Result<()> {
        let (content_type, filename) = mime_filename(path);
        let mut file = try!(File::open(path));
        self.write_stream(&mut file, name, filename, Some(content_type))
    }

    fn write_stream<S: Read>(&mut self, stream: &mut S, name: &str, filename: Option<&str>, content_type: Option<Mime>) -> io::Result<()> {
        // This is necessary to make sure it is interpreted as a file on the server end.
        let content_type = Some(content_type.unwrap_or_else(::mime_guess::octet_stream));

        chain_result! {
            self.write_field_headers(name, filename, content_type),
            io::copy(stream, &mut self.inner),
            Ok(()) 
        }
    }

    fn write_field_headers(&mut self, name: &str, filename: Option<&str>, content_type: Option<Mime>) 
    -> io::Result<()> {
        chain_result! {
            // Write the first boundary, or the boundary for the previous field.
            self.write_boundary(),
            { self.data_written = true; Ok(()) },
            write!(self.inner, "Content-Disposition: form-data; name=\"{}\"", name),
            filename.map(|filename| write!(self.inner, "; filename=\"{}\"", filename))
                .unwrap_or(Ok(())),
            content_type.map(|content_type| write!(self.inner, "\r\nContent-Type: {}", content_type))
                .unwrap_or(Ok(())),
            self.inner.write_all(b"\r\n\r\n")
        }
    }

    fn finish(mut self) -> io::Result<W> {
        if self.data_written {
            // Write two hyphens after the last boundary occurrence.
            try!(write!(self.inner, "\r\n--{}--", self.boundary));
        }

        Ok(self.inner)
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
