// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures_core::{Future, Poll, Stream, TryStream};
use std::task::Poll::*;

use std::rc::Rc;
use std::{str, mem};

use super::boundary::BoundaryFinder;
use super::Multipart;

use std::fmt;

use crate::BodyChunk;

// mod collect;
mod headers;

pub use self::headers::FieldHeaders;
pub(crate) use self::headers::ReadHeaders;

//pub use self::collect::{ReadTextField, TextField};
use futures_core::task::Context;
use std::pin::Pin;
use crate::server::{PushChunk, Error};
use crate::server::Error::Utf8;

/// A `Future` potentially yielding the next field in the multipart stream.
///
/// If there are no more fields in the stream, `Ok(None)` is returned.
///
/// See [`Multipart::next_field()`](../struct.Multipart.html#method.next_field) for usage.
pub struct NextField<'a, S: TryStream + 'a> {
    multipart: Option<Pin<&'a mut Multipart<S>>>,
    has_next_field: bool,
}

impl<'a, S: TryStream + 'a> NextField<'a, S> {
    pub(crate) fn new(multipart: Pin<&'a mut Multipart<S>>) -> Self {
        NextField {
            multipart: Some(multipart),
            has_next_field: false,
        }
    }

    fn multipart(&mut self) -> Option<Pin<&mut Multipart<S>>> {
        Some(self.multipart.as_mut()?.as_mut())
    }
}

impl<'a, S: 'a> Future for NextField<'a, S> where S: TryStream, S::Ok: BodyChunk, Error<S::Error>: From<S::Error> {
    type Output = super::Result<Option<Field<'a, S>>, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // since we can't use `?` with `Option<...>` in this context
        macro_rules! multipart {
            (get) => {
                if let Some(ref mut multipart) = self.multipart {
                    multipart.as_mut()
                } else {
                    return Ready(Ok(None));
                }
            };
            (take) => {
                if let Some(multipart) = self.multipart.take() {
                    multipart
                } else {
                    return Ready(Ok(None));
                }
            }
        }

        // `false` and `multipart = Some` means we haven't polled for next field yet
        self.has_next_field = self.has_next_field
            || ready!(multipart!(get).poll_has_next_field(cx)?);

        if !self.has_next_field {
            // end of stream, next `?` will return
            self.multipart = None;
        }

        Ready(Ok(Some(Field {
            headers: ready!(multipart!(get).poll_field_headers(cx)?),
            data: FieldData {
                multipart: multipart!(take)
            },
            _priv: ()
        })))
    }
}

/// A single field in a multipart stream.
///
/// The data of the field is provided as a `Stream` impl in the `data` field.
pub struct Field<'a, S: TryStream + 'a> {
    /// The headers of this field, including the name, filename, and `Content-Type`, if provided.
    pub headers: FieldHeaders,
    /// The data of this field in the request, represented as a stream of chunks.
    pub data: FieldData<'a, S>,
    _priv: (),
}

impl<S: TryStream> fmt::Debug for Field<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Field")
            .field("headers", &self.headers)
            .field("data", &"<FieldData>")
            .finish()
    }
}

/// The data of a field in a multipart stream, as a stream of chunks.
///
/// It may be read to completion via the `Stream` impl, or collected to a string with
/// `.read_to_string()`.
pub struct FieldData<'a, S: TryStream + 'a> {
    multipart: Pin<&'a mut Multipart<S>>,
}

impl<'a, S: TryStream + 'a> FieldData<'a, S> {

    /// Return a `Future` which yields the result of reading this field's data to a `String`.
    ///
    /// ### Note: UTF-8 Only
    /// Reading to a string using a non-UTF-8 charset is currently outside of the scope of this
    /// crate. Most browsers send form requests using the same charset as the page
    /// the form resides in, so as long as you only serve UTF-8 encoded pages, this would only
    /// realistically happen in one of two cases:
    ///
    /// * a non-browser client like cURL was specifically instructed by the user to
    /// use a non-UTF-8 charset, or:
    /// * the field is actually a file encoded in a charset that is not UTF-8
    /// (most likely Windows-1252 or UTF-16).
    pub fn read_to_string(self) -> ReadToString<'a, S> {
        ReadToString {
            multipart: self.multipart,
            string: String::new(),
            surrogate: None,
        }
    }
}

impl<S: TryStream> Stream for FieldData<'_, S>
where
    S::Ok: BodyChunk,
    Error<S::Error>: From<S::Error>
{
    type Item = super::Result<S::Ok, S::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        self.multipart.as_mut().poll_field_chunk(cx)
    }
}

pub struct ReadToString<'a, S: TryStream> {
    multipart: Pin<&'a mut Multipart<S>>,
    string: String,
    surrogate: Option<([u8; 3], u8)>,
}

impl<S: TryStream> Future for ReadToString<'_, S>
    where
        S::Ok: BodyChunk,
        Error<S::Error>: From<S::Error>
{
    type Output = super::Result<String, S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        while let Some(mut data) = ready!(self.multipart.as_mut().poll_field_chunk(cx)?) {
            if let Some((start, start_len)) = self.surrogate {
                assert!(start_len > 0 && start_len < 4, "start_len out of range: {:?}", start_len);

                let start_len = start_len as usize;

                let (width, needed) = if let Some(width) = utf8_char_width(start[0]) {
                    (width, width.checked_sub(start_len).expect("start_len >= width"))
                } else {
                    return Ready(fmt_err!("unexpected start of UTF-8 surrogate: {:X}", start[0]));
                };

                let mut surrogate = [0u8; 4];
                surrogate[..start_len].copy_from_slice(&start[..start_len]);
                surrogate[start_len .. width].copy_from_slice(data.slice(..needed));

                self.string.push_str(
                    str::from_utf8(&surrogate[..width])
                    .map_err(Utf8)?
                );

                let (_, rem) = data.split_into(width);
                data = rem;
            }

            match str::from_utf8(data.as_slice()) {
                Ok(s) => self.string.push_str(s),
                Err(e) => if e.error_len().is_some() {
                    // we encountered an invalid surrogate
                    return Ready(Err(Utf8(e)));
                } else {
                    self.string.push_str(unsafe {
                        // Utf8Error specifies that `..e.valid_up_to()` is valid UTF-8
                        str::from_utf8_unchecked(data.slice(..e.valid_up_to()))
                    });

                    let start_len = data.len() - e.valid_up_to();
                    let mut start = [0u8; 3];
                    start[..start_len].copy_from_slice(data.slice(e.valid_up_to()..));

                    // `e.valid_up_to()` is specified to be `[-1, -3]` of `data.len()`
                    self.surrogate = Some((start, start_len as u8));
                }
            }
        }

        if let Some((start, _)) = self.surrogate {
            ret_err!("incomplete UTF-8 surrogate: {:?}", start);
        }

        Ready(Ok(mem::replace(&mut self.string, String::new())))
    }
}

fn utf8_char_width(first: u8) -> Option<usize> {
    // simplification of the LUT here:
    // https://github.com/rust-lang/rust/blob/fe6d05a/src/libcore/str/mod.rs#L1565
    match first {
        // ASCII characters are one byte
        0x00 ..= 0x7F => Some(1),
        0xC2 ..= 0xDF => Some(2),
        0xE0 ..= 0xEF => Some(3),
        0xF0 ..= 0xF4 => Some(4),
        _ => None
    }
}

#[test]
fn assert_types_unpin() {
    use crate::test_util::assert_unpin;

    fn inner<'a, S: TryStream + 'a>() {
        assert_unpin::<FieldData<'a, S>>();
        assert_unpin::<ReadToString<'a, S>>();
    }
}
