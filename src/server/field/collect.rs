use futures::{Future, Stream};
use futures::Async::*;

use std::rc::Rc;
use std::str::Utf8Error;
use std::{fmt, str};


use server::boundary::BoundaryFinder;
use server::{Internal};

use {BodyChunk, StreamError};

use super::{FieldHeaders, FieldData};

use helpers::*;

enum ChunkStack<C> {
    Empty,
    One(C),
    Two(C, C),
}

impl<C> Default for ChunkStack<C> {
    fn default() -> Self {
        ChunkStack::Empty
    }
}

impl<C: BodyChunk> ChunkStack<C> {
    /// Push a chunk onto the stack
    fn push(&mut self, chunk: C) {
        use self::ChunkStack::*;

        *self = match replace_default(self) {
            Empty => One(chunk),
            // This way pushes and pops only have to move one value
            One(one) => Two(one, chunk),
            // print in stream order
            Two(one, two) => panic!("Chunk buffer full: [{}], [{}], [{}]",
                                    show_bytes(chunk.as_slice()), show_bytes(two.as_slice()),
                                    show_bytes(one.as_slice())),
        };
    }

    /// Pop a chunk from the stack
    fn pop(&mut self) -> Option<C> {
        use self::ChunkStack::*;

        match replace_default(self) {
            Empty => None,
            One(one) => { Some(one) },
            Two(one, two) => { *self = One(one); Some(two) }
        }
    }
}

impl<C: BodyChunk> fmt::Debug for ChunkStack<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::ChunkStack::*;

        match *self {
            Empty => write!(f, "<empty>"),
            One(ref one) => write!(f, "[{}]", show_bytes(one.as_slice())),
            Two(ref one, ref two) => write!(f, "[{}] + [{}]", show_bytes(one.as_slice()),
                                            show_bytes(two.as_slice())),
        }
    }
}

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
/// Decoding text in a different charset (except ASCII which is compatible with UTF-8) is,
/// currently, beyond the scope of this crate. However, as a convention, web browsers will send
/// `multipart/form-data` requests in the same charset as that of the document (page or frame)
/// containing the form, so if you only serve ASCII/UTF-8 pages then you won't have to worry
/// too much about decoding strange charsets.
///
/// ### Warning About Leaks
/// If this value or the contained `FieldData` is leaked (via `mem::forget()` or some
/// other mechanism), then the parent `Multipart` will never be able to yield the next field in the
/// stream. The task waiting on the `Multipart` will also never be notified, which, depending on the
/// event loop/reactor/executor implementation, may cause a deadlock.
#[derive(Default)]
pub struct ReadTextField<S: Stream> {
    stream: Option<S>,
    accum: String,
    chunks: ChunkStack<S::Item>,
    /// The headers for the original field, provided as a convenience.
    pub headers: Rc<FieldHeaders>,
    /// The length limit for the string, in bytes, to avoid potential DoS attacks from
    /// attackers running the server out of memory. If an incoming chunk is expected to push the
    /// string over this limit, an error is returned and the offending chunk is pushed back
    /// to the head of the stream.
    pub limit: usize,
}

// RFC on these numbers, they're pretty much arbitrary
const DEFAULT_LIMIT: usize = 65536; // 65KiB--reasonable enough for one text field, right?
const MAX_LIMIT: usize = 16_777_216; // 16MiB--highest sane value for one text field, IMO

pub fn read_text<S: Stream>(headers: Rc<FieldHeaders>, data: S) -> ReadTextField<S> {
    ReadTextField {
        headers, stream: Some(data), limit: DEFAULT_LIMIT, accum: String::new(),
        chunks: Default::default()
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
    pub fn into_data(self) -> Option<S> {
        self.stream
    }
}

impl<S: Stream> ReadTextField<S> where S::Item: BodyChunk {
    fn next_chunk(&mut self) -> PollOpt<S::Item, S::Error> {
        if let Some(chunk) = self.chunks.pop() {
            return ready(Some(chunk));
        }

        if let Some(ref mut stream) = self.stream {
            stream.poll()
        } else {
            ready(None)
        }
    }

    /// Try to poll for another chunk; if successful, return both of them, otherwise push the first
    /// chunk back.
    fn another_chunk(&mut self, first: S::Item) -> PollOpt<(S::Item, S::Item), S::Error> {
        match self.next_chunk() {
            Ok(Ready(Some(second))) => ready(Some((first, second))),
            Ok(Ready(None)) => ready(None),
            Ok(NotReady) => { self.chunks.push(first); not_ready() }
            Err(e) => { self.chunks.push(first); Err(e) },
        }
    }
}

impl<S: Stream> Future for ReadTextField<S> where S::Item: BodyChunk, S::Error: StreamError {
    type Item = TextField;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Self::Item, S::Error> {
        loop {
            let chunk = match try_ready!(self.next_chunk()) {
                Some(val) => val,
                _ => break,
            };

            // This also catches capacity overflows
            if self.accum.len().checked_add(chunk.len()).map_or(true, |len| len > self.limit) {
                self.chunks.push(chunk);
                ret_err!("text field {:?} exceeded limit of {} bytes", self.headers, self.limit);
            }

            // Try to convert the chunk to UTF-8 and append it to the accumulator
            let split_idx = match str::from_utf8(chunk.as_slice()) {
                Ok(s) => { self.accum.push_str(s); continue },
                Err(e) => if e.valid_up_to() > chunk.len() - 4 {
                    // this may just be a valid sequence split across two chunks
                    e.valid_up_to()
                } else {
                    // definitely was an invalid byte sequence
                    return utf8_err(e);
                },
            };

            let (valid, invalid) = chunk.split_at(split_idx);

            self.accum.push_str(str::from_utf8(valid.as_slice())
                .expect("a `StreamChunk` was UTF-8 before, now it's not"));

            // Recombine the cutoff UTF-8 sequence
            let char_width = utf8_char_width(invalid.as_slice()[0]);
            let needed_len =  char_width - invalid.len();

            // Get a second chunk or push the first chunk back
            let (first, second) = match try_ready!(self.another_chunk(invalid)) {
                Some(pair) => pair,
                // this also happens if we have some invalid bytes right at the end of the string
                // should be rare and the end result is the same
                None => ret_err!("unexpected end of stream while decoding a UTF-8 sequence"),
            };

            if second.len() < needed_len {
                ret_err!("got a chunk smaller than the {} byte(s) needed to finish \
                          decoding this UTF-8 sequence: {:?}",
                         needed_len, first.as_slice());
            }

            let over_limit = self.accum.len().checked_add(first.len())
                .and_then(|len| len.checked_add(second.len()))
                .map_or(true, |len| len > self.limit);

            if over_limit {
                // push chunks in reverse order
                self.chunks.push(second);
                self.chunks.push(first);
                ret_err!("text field {:?} exceeded limit of {} bytes", self.headers, self.limit);
            }

            let mut buf = [0u8; 4];

            // first.len() will be between 1 and 4 as guaranteed by `Utf8Error::valid_up_to()`
            buf[..first.len()].copy_from_slice(first.as_slice());
            buf[first.len()..].copy_from_slice(&second.as_slice()[..needed_len]);

            // if this fails we definitely got an invalid byte sequence
            str::from_utf8(&buf[..char_width]).map(|s| self.accum.push_str(s))
                .or_else(utf8_err)?;

            let (_, rem) = second.split_at(needed_len);

            if !rem.is_empty() {
                self.chunks.push(rem);
            }
        }

        // Optimization: free the `FieldData` so the parent `Multipart` can yield
        // the next field.
        self.stream = None;

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

#[inline]
fn utf8_char_width(b: u8) -> usize {
    return UTF8_CHAR_WIDTH[b as usize] as usize;
}
