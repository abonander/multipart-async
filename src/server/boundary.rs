// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
extern crate twoway;

use futures::{Async, Poll, Stream};

use std::cmp;
use std::borrow::Borrow;
use std::error::Error;
use std::io;
use std::mem;

use super::BodyChunk;

use self::State::*;

pub type PollOpt<T, E> = Poll<Option<T>, E>;

/// A struct implementing `Read` and `BufRead` that will yield bytes until it sees a given sequence.
// #[derive(Debug)]
pub struct BoundaryFinder<S: Stream> {
    stream: S,
    state: State<S::Item>,
    boundary: Box<[u8]>,
}

impl<S: Stream> BoundaryFinder<S> where S::Item: BodyChunk, S::Error: From<io::Error> {
    pub fn new<B: Into<Vec<u8>>>(stream: S, boundary: B) -> BoundaryFinder<S> {
        BoundaryFinder {
            stream: stream,
            state: State::Watching,
            boundary: boundary.into().into_boxed_slice(),
        }
    }

    pub fn body_chunk(&mut self) -> PollOpt<S::Item, S::Error> {
        macro_rules! try_ready_opt(
            ($try:expr) => (
                match $try {
                    Ok(Async::Ready(Some(val))) => val,
                    other => return other.into(),
                }
            );
            ($try:expr; $restore:expr) => (
                match $try {
                    Ok(Async::Ready(Some(val))) => val,
                    other => {
                        self.state = $restore;
                        return other.into();
                    }
                }
            )
        );

        loop {
            match self.state {
                Boundary(_) | BoundaryRem(_, _) | End => return ready(None),
                _ => ()
            }

            match mem::replace(&mut self.state, Watching) {
                Watching => {
                    let chunk = try_ready_opt!(self.stream.poll());
                    return self.check_chunk(chunk);
                },
                Remainder(rem) => return self.check_chunk(rem),
                Partial(mut partial) => {
                    let chunk = try_ready_opt!(self.stream.poll(); Partial(partial));
                    let needed_len = (self.boundary_size()).saturating_sub(partial.len());

                    if chunk.len() >= needed_len {
                        let (add, rem) = chunk.split_at(needed_len);
                        partial.extend_from_slice(add.as_slice());

                        if self.confirm_boundary(&partial) {
                            self.state = BoundaryRem(partial, rem);
                            return ready(None);
                        } else {
                            // This isn't the boundary we were looking for
                            self.state = Remainder(rem);
                            return ready(body_chunk::<S>(partial));
                        }
                    }

                    // *rare*: chunk didn't have enough bytes to verify
                    partial.extend_from_slice(chunk.as_slice());

                    if !self.boundary.starts_with(&partial) {
                        // wasn't our boundary
                        self.state = Watching;
                        return ready(body_chunk::<S>(partial));
                    }

                    // wait for next chunk
                    self.state = Partial(partial);
                },
                _ => unreachable!("invalid state"),
            }
        }
    }

    fn check_chunk(&mut self, chunk: S::Item) -> PollOpt<S::Item, S::Error> {
        if let Some(idx) = self.find_boundary(&chunk) {
            // Back up so we don't yield the CRLF before the boundary
            let idx = idx.saturating_sub(2);

            let (ret, rem) = chunk.split_at(idx);

            self.state = if rem.len() < self.boundary_size() {
                // Either partial boundary, or boundary but not the two bytes after it
                Partial(rem.into_vec())
            } else {
                Boundary(rem)
            };

            ready(ret)
        } else {
            ready(chunk)
        }
    }

    fn find_boundary(&self, chunk: &S::Item) -> Option<usize> {
        twoway::find_bytes(chunk.as_slice(), &self.boundary)
            .or_else(|| partial_rmatch(chunk.as_slice(), &self.boundary))
    }

    fn maybe_boundary(&self, bytes: &[u8]) -> bool {
        (bytes.len() >= 2 && self.boundary.starts_with(&bytes[2..]))
            || self.boundary.starts_with(bytes)
    }

    fn confirm_boundary(&self, bytes: &[u8]) -> bool {
        (bytes.len() >= 2 && bytes[2..].starts_with(&self.boundary))
            || bytes.starts_with(&self.boundary)
    }

    #[doc(hidden)]
    pub fn consume_boundary(&mut self) -> Poll<bool, S::Error> {
        while try_ready!(self.body_chunk()).is_some() {}

        let boundary = match mem::replace(&mut self.state, Watching) {
            Boundary(boundary) => boundary.into_vec(),
            BoundaryRem(boundary, rem) => {
                self.state = Remainder(rem);
                boundary
            },
            _ => unreachable!("invalid state"),
        };

        let len = boundary.len();

        if len <= self.boundary_size() {
            return error(format!("boundary sequence too short: {}", String::from_utf8_lossy(&boundary)));
        }

        let is_end = boundary.ends_with(b"--");

        if !is_end && !boundary.ends_with(b"\r\n") {
            error!("unexpected bytes after boundary: {:?}", &boundary[len - 2 ..]);
        }

        ready(is_end)
    }

    /// The necessary size to verify a boundary, including the potential CRLF before, and the
    /// CRLF / "--" afterward
    fn boundary_size(&self) -> usize {
        self.boundary.len() + 4
    }
}

enum State<B> {
    /// Watching for next boundary
    Watching,
    /// Partial boundary, accumulating test bytes to the vector
    Partial(Vec<u8>),
    Boundary(B),
    BoundaryRem(Vec<u8>, B),
    /// The remains of a chunk after processing
    Remainder(B),
    End,
}

fn body_chunk<S: Stream>(vec: Vec<u8>) -> S::Item where S::Item: BodyChunk {
    BodyChunk::from_vec(vec)
}

fn ready<R, E, T: Into<R>>(val: T) -> Poll<R, E> {
    Ok(Async::Ready(val.into()))
}

fn not_ready<T, E>() -> Poll<T, E> {
    Ok(Async::NotReady)
}

fn error<T, E: Into<Box<Error + Send + Sync>>, E_: From<io::Error>>(e: E) -> Poll<T, E_> {
    Err(io::Error::new(io::ErrorKind::Other, e).into())
}

/// Check if `needle` is cut off at the end of `haystack`, and if so, its index
fn partial_rmatch(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if haystack.is_empty() || needle.is_empty() { return None; }
    if haystack.len() < needle.len() { return None; }

    let trim_start = haystack.len() - (needle.len() - 1);

    let idx = try_opt!(twoway::find_bytes(&haystack[trim_start..], &needle[..1])) + trim_start;

    // If the rest of `haystack` matches `needle`, then we have our partial match
    if haystack[idx..].iter().zip(needle).all(|(l, r)| l == r) {
        Some(idx)
    } else {
        None
    }
}

#[cfg(test)]
mod test {
    use super::BoundaryFinder;

    use futures::{Future, Stream};
    use futures::Async::*;

    use std::io;
    use std::io::prelude::*;

    const BOUNDARY: &'static str = "--boundary";

    struct MockStream<'s> {
        stream: &'s mut [(&'static [u8], usize)],
    }

    fn assert_stream_eq<S1: Stream, S2: Stream<Error = S1::Error>>(mut left: S1, mut right: S2) where S1::Item: PartialEq<S2::Item> {
        loop {
            let left_item = loop {
                match left.poll().expect("Error from left stream") {
                    Ready(item) => break item,
                    _ => (),
                }
            };

            let right_item = loop {
                match right.poll().expect("Error from right stream") {
                    Ready(item) => break item,
                    _ => (),
                }
            };

            if left_item.is_none() && right_item.is_none() { return; }

            assert_eq!(left_item, right_item);
        }
    }

    #[test]
    fn test_boundary() {
        let _ = ::env_logger::init();        
        debug!("Testing boundary (no split)");

        let src = &mut TEST_VAL.as_bytes();
        let reader = BoundaryFinder::from_reader(src, BOUNDARY);
        
        test_boundary_reader(reader);        
    }

    struct SplitReader<'a> {
        left: &'a [u8],
        right: &'a [u8],
    }

    impl<'a> SplitReader<'a> {
        fn split(data: &'a [u8], at: usize) -> SplitReader<'a> {
            let (left, right) = data.split_at(at);

            SplitReader { 
                left: left,
                right: right,
            }
        }
    }

    impl<'a> Read for SplitReader<'a> {
        fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
            fn copy_bytes_partial(src: &mut &[u8], dst: &mut [u8]) -> usize {
                src.read(dst).unwrap()
            }

            let mut copy_amt = copy_bytes_partial(&mut self.left, dst);

            if copy_amt == 0 {
                copy_amt = copy_bytes_partial(&mut self.right, dst)
            };

            Ok(copy_amt)
        }
    }

    #[test]
    fn test_split_boundary() {
        let _ = ::env_logger::init();        
        debug!("Testing boundary (split)");
        
        // Substitute for `.step_by()` being unstable.
        for split_at in (0 .. TEST_VAL.len()).filter(|x| x % 2 != 0) {
            debug!("Testing split at: {}", split_at);

            let src = SplitReader::split(TEST_VAL.as_bytes(), split_at);
            let reader = BoundaryFinder::from_reader(src, BOUNDARY);
            test_boundary_reader(reader);
        }

    }

    fn test_boundary_reader<R: Read>(mut reader: BoundaryFinder<R>) {
        let ref mut buf = String::new();    

        debug!("Read 1");
        let _ = reader.read_to_string(buf).unwrap();
        assert!(buf.is_empty(), "Buffer not empty: {:?}", buf);
        buf.clear();

        debug!("Consume 1");
        reader.consume_boundary().unwrap();

        debug!("Read 2");
        let _ = reader.read_to_string(buf).unwrap();
        assert_eq!(buf, "\r\ndashed-value-1");
        buf.clear();

        debug!("Consume 2");
        reader.consume_boundary().unwrap();

        debug!("Read 3");
        let _ = reader.read_to_string(buf).unwrap();
        assert_eq!(buf, "\r\ndashed-value-2");
        buf.clear();

        debug!("Consume 3");
        reader.consume_boundary().unwrap();

        debug!("Read 4");
        let _ = reader.read_to_string(buf).unwrap();
        assert_eq!(buf, "--");
    }
}
