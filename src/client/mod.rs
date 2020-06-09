// Copyright 2017-2019 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! The client-side abstraction for multipart requests. Enabled with the `client` feature (on by
//! default).
//!
//! Use this when sending POST requests with files to a server.

use http::HeaderValue;
use rand::distributions::{Alphanumeric, Distribution};
use tokio::io::AsyncWrite;

use crate::client::writer::MultipartWriter;

pub mod writer;

const BOUNDARY_LEN: usize = 32;

pub struct MultipartRequest {
    boundary: String,
}

impl MultipartRequest {
    /// Start building a new `multipart/form-data` request.
    pub fn new() -> Self {
        let mut boundary = String::with_capacity(BOUNDARY_LEN);
        boundary.extend(
            Alphanumeric
                .sample_iter(rand::thread_rng())
                .take(BOUNDARY_LEN),
        );

        MultipartRequest { boundary }
    }

    /// Get the value of the `Content-Type` header to be sent to the server.
    pub fn get_content_type(&self) -> HeaderValue {
        format!("multipart/form-data; boundary={}", self.boundary)
            .parse()
            .expect("this should be a valid header value")
    }

    /// Wrap a `AsyncWrite` impl.
    pub fn wrap_writer<W: AsyncWrite + Unpin>(self, writer: W) -> MultipartWriter<W> {
        MultipartWriter::new(writer, self.boundary)
    }
}

#[test]
fn test_multipart_get_content_type() {
    let request = MultipartRequest {
        boundary: "boundary".to_string(),
    };

    assert_eq!(
        request.get_content_type(),
        "multipart/form-data; boundary=boundary"
    );
}
