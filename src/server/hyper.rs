// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! Server-side integration with [Hyper](https://github.com/hyperium/hyper).
//! Enabled with the `hyper` feature (on by default).
use hyper::header::ContentType;
use hyper::method::Method;
use hyper::server::Request as HyperRequest;

use mime::{Mime, TopLevel, SubLevel, Attr, Value};

use super::Multipart;

impl Multipart {
    /// ###Feature: `hyper`
    pub fn from_request<T>(req: &HyperRequest<T>) -> Option<Self> {
        if let Method::Post = *req.method() {
            get_boundary(req).map(Self::new)
        } else {
            None
        }
    }
}

fn get_boundary<'a, T>(req: &'a HyperRequest<T>) -> Option<&'a str> {
    req.headers().get::<ContentType>()
        .and_then(|&ContentType(ref mime)| get_boundary_mime(mime))
}

fn get_boundary_mime(mime: &Mime) -> Option<&str> {
    if let Mime(TopLevel::Multipart, SubLevel::FormData, _) = *mime {
        mime.get_param(Attr::Boundary)
            .and_then(|val| match *val {
                Value::Ext(ref val) => Some(&**val),
                _ => None
            })
    } else {
        None
    }
}
