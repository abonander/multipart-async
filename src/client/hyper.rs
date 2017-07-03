// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! Client-side integration with [Hyper](https://github.com/hyperium/hyper). 
//! Enabled with the `hyper` feature (on by default).
//!
//! Contains `impl HttpRequest for Request<Fresh>` and `impl HttpStream for Request<Streaming>`.
//!
//! Also see: [`lazy::Multipart::client_request()`](../lazy/struct.Multipart.html#method.client_request)
//! and [`lazy::Multipart::client_request_mut()`](../lazy/struct.Multipart.html#method.client_request_mut)
//! (adaptors for `hyper::client::RequestBuilder`).
use hyper::header::{ContentType, ContentLength};
use hyper::Method;
use mime::{Mime, TopLevel, SubLevel, Attr, Value};

use hyper::client::Request as HyperRequest;

use super::Request;

impl<'req> Request for HyperRequest<'req> {
    fn set_method(&mut self) {
        self.set_method(Method::Post);
    }

    fn set_boundary(&mut self, boundary: &str) {
        self.headers_mut().set(content_type(boundary));
    }

    fn set_content_len(&mut self, content_len: u64) {
        self.headers_mut().set(ContentLength(content_len));
    }
}


/// Create a `Content-Type: multipart/form-data;boundary={bound}`
pub fn content_type(bound: &str) -> ContentType {
    ContentType(multipart_mime(bound))
}

fn multipart_mime(bound: &str) -> Mime {
    Mime(
        TopLevel::Multipart, SubLevel::Ext("form-data".into()),
        vec![(Attr::Ext("boundary".into()), Value::Ext(bound.into()))]
    )         
}
