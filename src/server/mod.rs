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
use futures::task::{self, Context};
use futures::{Future, Poll, Stream};

use std::cell::Cell;
use std::rc::Rc;

use self::boundary::BoundaryFinder;

use crate::{BodyChunk, StreamError};

macro_rules! try_opt (
    ($expr:expr) => (
        match $expr {
            Some(val) => val,
            None => return None,
        }
    )
);

macro_rules! ret_err (
    ($($args:tt)+) => (
            return fmt_err!($($args)+);
    )
);

macro_rules! fmt_err(
    ($string:expr) => (
        crate::helpers::error($string)
    );
    ($string:expr, $($args:tt)*) => (
        crate::helpers::error(format!($string, $($args)*))
    );
);

mod boundary;
mod field;

use crate::helpers::*;

use self::field::ReadHeaders;

pub use self::field::{Field, FieldData, FieldHeaders};
// pub use self::field::{ReadTextField, TextField};

#[cfg(feature = "hyper")]
mod hyper;

#[cfg(feature = "hyper")]
pub use self::hyper::{MinusBody, MultipartService};
use std::pin::Pin;

#[cfg(any(test, feature = "fuzzing"))]
pub(crate) mod fuzzing {
    pub(crate) use super::boundary::BoundaryFinder;
    pub(crate) use super::field::ReadHeaders;
}

/// The server-side implementation of `multipart/form-data` requests.
pub struct Multipart<S: TryStream> {
    inner: PushChunk<BoundaryFinder<S>, S::Ok>,
    read_hdr: ReadHeaders,
    consumed: bool,
}

// Q: why can't we just wrap up these bounds into a trait?
// A: https://github.com/rust-lang/rust/issues/24616#issuecomment-112065997
// (The workaround mentioned in a later comment doesn't seem to be worth the added complexity)
impl<S> Multipart<S>
where
    S: TryStream,
    S::Ok: BodyChunk,
    S::Error: StreamError,
{
    unsafe_pinned!(inner: PushChunk<BoundaryFinder<S>, S::Ok>);
    unsafe_unpinned!(read_hdr: ReadHeaders);
    unsafe_unpinned!(consumed: bool);

    /// Construct a new `Multipart` with the given body reader and boundary.
    ///
    /// This will add the requisite `--` and CRLF (`\r\n`) to the boundary as per
    /// [IETF RFC 7578 section 4.1](https://tools.ietf.org/html/rfc7578#section-4.1).
    pub fn with_body<B: Into<String>>(stream: S, boundary: B) -> Self {
        let mut boundary = boundary.into();
        boundary.insert_str(0, "--");

        debug!("Boundary: {}", boundary);

        Multipart {
            inner: PushChunk::new(BoundaryFinder::new(stream, boundary)),
            read_hdr: ReadHeaders::default(),
            consumed: false,
        }
    }

    pub fn poll_next_field_headers(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
    ) -> PollOpt<FieldHeaders, S::Error> {
        if !ready!(self.as_mut().inner().stream().consume_boundary(cx)?) {
            return Poll::Ready(None);
        }

        unsafe {
            let this = self.as_mut().get_unchecked_mut();
            this.read_hdr
                .read_headers(Pin::new_unchecked(&mut this.inner), cx)
        }
    }

    pub fn poll_body_chunk(self: Pin<&mut Self>, cx: &mut Context) -> PollOpt<S::Ok, S::Error> {
        if !self.read_hdr.is_reading_headers() {
            self.inner().poll_next(cx)
        } else {
            Poll::Ready(None)
        }
    }

    pub fn poll_field(mut self: Pin<&mut Self>, cx: &mut Context) -> PollOpt<Field<S>, S::Error> {
        if let Some(headers) = ready!(self.as_mut().poll_next_field_headers(cx)?) {
            ready_ok(field::new_field(headers, self.inner()))
        } else {
            Poll::Ready(None)
        }
    }
}

/// An extension trait for requests which may be multipart.
pub trait RequestExt: Sized {
    /// The success type, may contain `Multipart` or something else.
    type Multipart;

    /// Convert `Self` into `Self::Multipart` if applicable.
    fn into_multipart(self) -> Result<Self::Multipart, Self>;
}

/// Struct wrapping a stream which allows a chunk to be pushed back to it to be yielded next.
pub(crate) struct PushChunk<S, T> {
    stream: S,
    pushed: Option<T>,
}

impl<S, T> PushChunk<S, T> {
    unsafe_pinned!(stream: S);
    unsafe_unpinned!(pushed: Option<T>);

    pub(crate) fn new(stream: S) -> Self {
        PushChunk {
            stream,
            pushed: None,
        }
    }
}

impl<S: TryStream> PushChunk<S, S::Ok> where S::Ok: BodyChunk {
    fn push_chunk(mut self: Pin<&mut Self>, chunk: S::Ok) {
        if let Some(pushed) = self.as_mut().pushed() {
            error!(
                "pushing excess chunk: \"{}\" already pushed chunk: \"{}\"",
                show_bytes(chunk.as_slice()),
                show_bytes(pushed.as_slice())
            );
        }

        *self.as_mut().pushed() = Some(chunk);
    }
}

impl<S: TryStream> Stream for PushChunk<S, S::Ok> {
    type Item = Result<S::Ok, S::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        if let Some(pushed) = self.as_mut().pushed().take() {
            return Poll::Ready(Some(Ok(pushed)));
        }

        self.stream().try_poll_next(cx)
    }
}

#[cfg(test)]
mod test {
    use futures::prelude::*;
    use super::Multipart;
    use crate::test_util::mock_stream;
    use crate::server::FieldHeaders;

    const BOUNDARY: &str = "boundary";

    #[test]
    fn test_empty_body() {
        let _ = ::env_logger::try_init();
        let multipart = Multipart::with_body(
            mock_stream(&[]),
            BOUNDARY
        );
        pin_mut!(multipart);
        ready_assert_eq!(|cx| multipart.as_mut().poll_next_field_headers(cx), None);
    }

    #[test]
    fn test_no_headers() {
        let _ = ::env_logger::try_init();
        let multipart = Multipart::with_body(
            mock_stream(&[b"--boundary", b"\r\n", b"\r\n", b"--boundary--"]),
            BOUNDARY
        );
        pin_mut!(multipart);
        ready_assert_eq!(|cx| multipart.as_mut().poll_next_field_headers(cx), None);
    }

    #[test]
    fn test_single_field() {
        let _ = ::env_logger::try_init();
        let multipart = Multipart::with_body(
            mock_stream(&[
                b"--boundary\r", b"\n",
                b"Content-Disposition:",
                b" form-data; name=",
                b"\"foo\"",
                b"\r\n\r\n",
                b"field data",
                b"\r", b"\n--boundary--"
            ]),
            BOUNDARY
        );
        pin_mut!(multipart);

        ready_assert_eq!(
            |cx| multipart.as_mut().poll_next_field_headers(cx),
            Some(Ok(FieldHeaders {
                name: "foo".into(),
                filename: None,
                content_type: None,
                ext_headers: Default::default(),
                _backcompat: (),
            }))
        );
    }
}
