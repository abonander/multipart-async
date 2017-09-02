// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! Server-side integration with [Hyper](https://github.com/hyperium/hyper).
//! Enabled with the `hyper` feature (on by default).
use bytes::Bytes;
use hyper::header::ContentType;

pub use hyper::{Body, Chunk, Error, Headers, HttpVersion, Method, Request, Uri};

use mime::{self, Mime, Name};

use std::str::Utf8Error;

use super::{Multipart, BodyChunk, StreamError};

pub trait HyperReqExt: Sized {
    fn into_multipart(self) -> Result<(Multipart<Body>, MinusBody), Self>;
}

impl HyperReqExt for Request {
    fn into_multipart(self) -> Result<(Multipart<Body>, MinusBody), Self> {
        if let Some(boundary) = get_boundary(&self) {
            let (body, minus_body) = MinusBody::from_req(self);
            Ok((Multipart::with_body(body, boundary), minus_body))
        } else {
            Err(self)
        }
    }
}

/// A deconstructed `server::Request` with the body extracted.
#[allow(missing_docs)]
#[derive(Debug)]
pub struct MinusBody {
    pub method: Method,
    pub uri: Uri,
    pub version: HttpVersion,
    pub headers: Headers,
}

impl MinusBody {
    fn from_req(req: Request<Body>) -> (Body, Self) {
        let (method, uri, version, headers, body) = req.deconstruct();
        (body, MinusBody { method, uri, version, headers })
    }
}

fn get_boundary(req: &Request<Body>) -> Option<String> {
    req.headers().get::<ContentType>()
        .and_then(|&ContentType(ref mime)| get_boundary_mime(mime))
}

fn get_boundary_mime(mime: &Mime) -> Option<String> {
    if *mime == mime::MULTIPART_FORM_DATA {
        mime.get_param(mime::BOUNDARY).map(|n|n.as_ref().into())
    } else {
        None
    }
}

impl BodyChunk for Chunk {
    #[inline]
    fn split_at(self, idx: usize) -> (Self, Self) {
        let (first, second) = Bytes::from(self).split_at(idx);
        (first.into(), second.into())
    }

    #[inline]
    fn as_slice(&self) -> &[u8] {
        self
    }
}

impl StreamError for Error {
    fn from_utf8(err: Utf8Error) -> Self {
        err.into()
    }
}
