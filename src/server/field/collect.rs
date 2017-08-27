use futures::{Future, Stream};
use futures::Async::*;

use std::rc::Rc;
use std::{fmt, str};

use server::boundary::BoundaryFinder;
use server::{Internal, BodyChunk, StreamError};

use super::{FieldHeaders, FieldData};

use helpers::*;

/// The result of reading a `Field` to text.
#[derive(Clone, Debug)]
pub struct TextField {
    /// The headers for the original field, provided as a convenience.
    pub headers: Rc<FieldHeaders>,
    /// The text of the field.
    pub text: String,
}

/// A `Future` which attempts to read a field's data to a string.
///
/// ### Charset
/// For simplicity, the default UTF-8 character set is assumed, as defined in
/// [IETF RFC 7578 Section 5.1.2](https://tools.ietf.org/html/rfc7578#section-5.1.2).
/// If the field body cannot be decoded as UTF-8, an error is returned.
///
/// Decoding text in a different charset (except ASCII and ISO/IEC-8859-1 which
/// are compatible with UTF-8) is, currently, beyond the scope of this crate.
///
/// ### Warning About Leaks
/// If this value or the contained `FieldData` is leaked (via `mem::forget()` or some
/// other mechanism), then the parent `Multipart` will never be able to yield the next field in the
/// stream. The task waiting on the `Multipart` will also never be notified, which, depending on the
/// event loop/reactor/executor implementation, may cause a deadlock.
#[derive(Default)]
pub struct ReadTextField<S: Stream> {
    data: Option<FieldData<S>>,
    accum: String,
    /// The headers for the original field, provided as a convenience.
    pub headers: Rc<FieldHeaders>,
    /// The limit for the string, in bytes, to avoid potential DoS attacks from
    /// attackers running the server out of memory. If an incoming chunk is expected to push the
    /// string over this limit, an error is returned and the offending chunk is pushed back
    /// to the head of the stream.
    pub limit: usize,
}

// RFC on these numbers, they're pretty much arbitrary
const DEFAULT_LIMIT: usize = 65536; // 65KiB--reasonable enough for one field, right?
const MAX_LIMIT: usize = 16_777_216; // 16MiB--highest sane value for one field, IMO

pub fn read_text<S: Stream>(data: FieldData<S>) -> ReadTextField<S> {
    ReadTextField {
        headers: data.headers.clone(), data: Some(data), limit: DEFAULT_LIMIT, accum: String::new()
    }
}

impl<S: Stream> ReadTextField<S> {
    /// Set the length limit, in bytes, for the collected text. If an incoming chunk is expected to
    /// push the string over this limit, an error is returned and the offending chunk is pushed back
    /// to the head of the stream.
    ///
    /// Setting a value higher than a few megabytes is not recommended as it could allow an attacker
    /// to DoS the server by running it out of memory, causing it to panic on allocation or spend
    /// forever swapping pagefiles to disk. Remember that this limit is only for a single field
    /// as well.
    ///
    /// Setting this to `usize::MAX` is equivalent to removing the limit as the string
    /// would overflow its capacity value anyway.
    pub fn limit(self, limit: usize) -> Self {
        Self { limit, .. self}
    }

    /// Soft max limit if the default isn't large enough.
    ///
    /// Going higher than this is allowed, but not recommended.
    pub fn limit_max(self) -> Self {
        self.limit(MAX_LIMIT)
    }

    /// Take the text that has been collected so far, leaving an empty string in its place.
    ///
    /// If the length limit was hit, this allows the field to continue being read.
    pub fn take_string(&mut self) -> String {
        replace_default(&mut self.accum)
    }

    /// The text that has been collected so far.
    pub fn ref_text(&self) -> &str {
        &self.accum
    }

    /// Destructure this future, taking the internal `FieldData` instance back.
    ///
    /// Will be `None` if the field was read to completion, because the internal `FieldData`
    /// instance is dropped afterwards to allow the parent `Multipart` to immediately start
    /// working on the next field.
    pub fn into_data(self) -> Option<FieldData<S>> {
        self.data
    }
}

impl<S: Stream> Future for ReadTextField<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = TextField;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Self::Item, S::Error> {
        loop {
            let data = match self.data {
                Some(ref mut data) => data,
                None => return not_ready(),
            };

            let mut stream = data.stream_mut();

            let chunk = match try_ready!(stream.body_chunk()) {
                Some(val) => val,
                _ => break,
            };

            // This also catches capacity overflows
            if self.accum.len().saturating_add(chunk.len()) > self.limit {
                stream.push_chunk(chunk);
                return Err(StreamError::from_string(format!("Text field {:?} exceeded limit \
                                                                  of {} bytes", self.headers, self.limit)));
            }

            // Try to convert the chunk to UTF-8 and append it to the accumulator
            let split_idx = match str::from_utf8(chunk.as_slice()) {
                Ok(s) => { self.accum.push_str(s); continue },
                Err(e) => match e.error_len() {
                    // a non-null `error_len` means there was an invalid byte sequence
                    Some(_) => return Err(StreamError::from_utf8(e)),
                    // otherwise, it just means that there was a byte sequence cut off by a
                    // chunk boundary
                    None => e.valid_up_to(),
                },
            };

            let (valid, invalid) = chunk.split_at(split_idx);

            self.accum.push_str(str::from_utf8(valid.as_slice())
                .expect("a `StreamChunk` was UTF-8 before, now it's not"));

            // Recombine the cutoff UTF-8 sequence
            let needed_len = utf8_char_width(invalid.as_slice()[0]) - invalid.len();

            // Get a second chunk or push the first chunk back
            let (first, second) = match try_ready!(stream.another_chunk(invalid)) {
                Some(pair) => pair,
                None => return error("unexpected end of stream while decoding a UTF-8 sequence"),
            };

            if second.len() < needed_len {
                return error(format!("got a chunk smaller than the {} byte(s) needed to finish \
                                      decoding this UTF-8 sequence: {:?}",
                                     needed_len, first.as_slice()));
            }

            if self.accum.len().saturating_add(first.len()).saturating_add(second.len()) > self.limit {
                // push in reverse order
                stream.push_chunk(second);
                stream.push_chunk(first);
                return Err(StreamError::from_string(format!("Text field {:?} exceeded limit \
                                                             of {} bytes",
                                                            self.headers, self.limit)));
            }

            let mut buf = [0u8; 4];

            // first.len() will be between 1 and 4 as guaranteed by `FromUtf8Error::valid_up_to()`
            buf[..first.len()].copy_from_slice(first.as_slice());
            buf[first.len()..].copy_from_slice(&second.as_slice()[.. needed_len]);

            let split_idx = match str::from_utf8(&buf) {
                Ok(s) => { self.accum.push_str(s); needed_len },
                Err(e) => match e.error_len() {
                    Some(_) => return utf8_err(e),
                    None => e.valid_up_to(),
                }
            };

            let (_, rem) = second.split_at(split_idx);

            if !rem.is_empty() {
                stream.push_chunk(rem);
            }
        }

        // Optimization: free the `FieldData` so the parent `Multipart` can yield
        // the next field.
        self.data = None;

        ready(TextField {
            headers: self.headers.clone(),
            text: self.take_string(),
        })
    }
}

impl<S: Stream> fmt::Debug for ReadTextField<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ReadFieldText")
            .field("accum", &self.accum)
            .field("headers", &self.headers)
            .field("limit", &self.limit)
            .finish()
    }
}

// Below lifted from https://github.com/rust-lang/rust/blob/1.19.0/src/libcore/str/mod.rs#L1461-L1485
// because they're being selfish with their UTF-8 implementation internals
static UTF8_CHAR_WIDTH: [u8; 256] = [
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, // 0x1F
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, // 0x3F
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, // 0x5F
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, // 0x7F
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, // 0x9F
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, // 0xBF
    0,0,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
    2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2, // 0xDF
    3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3, // 0xEF
    4,4,4,4,4,0,0,0,0,0,0,0,0,0,0,0, // 0xFF
];

fn utf8_char_width(b: u8) -> usize {
    return UTF8_CHAR_WIDTH[b as usize] as usize;
}
