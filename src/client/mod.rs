//! The client-side abstraction for multipart requests. Enabled with the `client` feature (on by
//! default).
//!
//! Use this when sending POST requests with files to a server.

use http::HeaderValue;
use rand::distributions::{Alphanumeric, Distribution};
use tokio_io::AsyncWrite;

use crate::client::streaming::MultipartWriter;

pub mod streaming;

const BOUNDARY_LEN: usize = 32;

pub struct MultipartRequest {
    boundary: String,
}

impl MultipartRequest {
    /// Start building a new `multipart/form-data` request.
    pub fn new() -> Self {
        let mut boundary = String::with_capacity(BOUNDARY_LEN);
        boundary.extend(Alphanumeric.sample_iter(rand::thread_rng()).take(BOUNDARY_LEN));

        MultipartRequest {
            boundary
        }
    }

    /// Get the value of the `Content-Type` header to be sent to the server.
    pub fn get_content_type(&self) -> HeaderValue {
        format!("multipart/form-data; boundary={}", self.boundary)
            .parse().expect("this should be a valid header value")
    }

    /// Wrap a `AsyncWrite` impl.
    pub fn wrap_writer<W: AsyncWrite + Unpin>(self, writer: W) -> MultipartWriter<W> {
        MultipartWriter::new(writer, self.boundary)
    }
}
