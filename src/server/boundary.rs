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
                Boundary(_, _) | BoundaryVec(_, _) | End => return ready(None),
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
                            self.state = BoundaryVec(partial, rem);
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
            let (idx, len) = if idx < 2 {
                // If there's not enough bytes before the boundary, don't back up
                (idx, self.boundary.len() + 2)
            } else {
                (idx - 2, self.boundary_size())
            };

            let (ret, rem) = chunk.split_at(idx);

            self.state = if rem.len() < len {
                // Either partial boundary, or boundary but not the two bytes after it
                Partial(rem.into_vec())
            } else {
                let (bnd, rem) = rem.split_at(len);

                Boundary(bnd, rem)
            };

            if !ret.is_empty() { ready(ret) } else { ready (None) }
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

    pub fn consume_boundary(&mut self) -> Poll<bool, S::Error> {
        while try_ready!(self.body_chunk()).is_some() {}

        match mem::replace(&mut self.state, Watching) {
            Boundary(bnd, rem) => self.check_boundary(bnd.as_slice(), rem),
            BoundaryVec(bnd, rem) => self.check_boundary(&bnd, rem),
            _ => unreachable!("invalid state"),
        }
    }

    fn check_boundary(&mut self, boundary: &[u8], rem: S::Item) -> Poll<bool, S::Error> {
        self.state = Remainder(rem);

        trace!("Boundary found: {}", String::from_utf8_lossy(boundary));

        let len = boundary.len();

        if len <= self.boundary_size() {
            return error(format!("boundary sequence too short: {}", String::from_utf8_lossy(boundary)));
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

impl<S: Stream> Stream for BoundaryFinder<S> where S::Item: BodyChunk, S::Error: From<io::Error> {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.body_chunk()
    }
}

enum State<B> {
    /// Watching for next boundary
    Watching,
    /// Partial boundary, accumulating test bytes to the vector
    Partial(Vec<u8>),
    Boundary(B, B),
    BoundaryVec(Vec<u8>, B),
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
    use super::{BoundaryFinder, ready, not_ready};

    use server::BodyChunk;

    use futures::{Future, Stream, Poll};
    use futures::Async::*;

    use std::fmt::Debug;
    use std::io;
    use std::io::prelude::*;

    const BOUNDARY: &'static str = "--boundary";

    struct MockStream {
        stream: Vec<(&'static [u8], usize)>,
        curr: usize,
    }

    impl MockStream {
        fn new(stream: Vec<(&'static [u8], usize)>) -> Self {
            MockStream {
                stream: stream,
                curr: 0,
            }
        }
    }

    impl Stream for MockStream {
        type Item = Vec<u8>;
        type Error = io::Error;

        fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
            let curr = self.curr;

            if curr >= self.stream.len() {
                return ready(None);
            }

            if self.stream[curr].1 > 0 {
                self.stream[curr].1 -= 1;
                return not_ready();
            }

            self.curr += 1;

            ready(self.stream[curr].0.to_owned())
        }
    }

    macro_rules! mock_finder (
        ($($input:tt)*) => (
            BoundaryFinder::new(mock!($($input)*), BOUNDARY)
        );
    );

    macro_rules! mock (
        ($($item:expr $(,$wait:expr)*);*) => (
            MockStream::new(vec![$(mock_item!($item $(, $wait)*)),*])
        )
    );

    macro_rules! mock_item (
        ($item:expr) => (($item, 0));
        ($item:expr, $($wait:expr)*) => (($item, $($wait)*));
    );

    fn assert_stream_eq<S: Stream, T>(mut left: S, right: &[T]) where S::Item: PartialEq<T>, S::Error: Debug, S::Item: Debug, T: Debug{
        let mut right = right.iter();

        loop {
            let left_item = loop {
                match left.poll().expect("Error from stream") {
                    Ready(item) => break item,
                    _ => (),
                }
            };

            let right_item = right.next();

            match (left_item, right_item) {
                (Some(ref left), Some(ref right)) if left == *right => (),
                (None, None) => (),
                (left, right) => panic!("Failed assertion: `{:?} == {:?}`", left, right),
            }
        }
    }

    fn assert_boundary<S: Stream<Item = Vec<u8>>>(finder: &mut BoundaryFinder<S>, is_end: bool) where S::Error: From<io::Error> + Debug {
        loop {
            match finder.body_chunk().expect("Error from BoundaryFinder") {
                Ready(Some(chunk)) => panic!("Unexpected chunk from BoundaryFinder: {}", String::from_utf8_lossy(chunk.as_slice())),
                Ready(None) => break,
                _ => (),
            }
        }

        loop {
            match finder.consume_boundary().expect("Error from BoundaryFinder") {
                Ready(val) => assert_eq!(val, is_end, "Found wrong kind of boundary"),
                _ => (),
            }
        }
    }

    #[test]
    fn simple_test() {
        let _ = ::env_logger::init();

        let mut finder = mock_finder! (
            b"--boundary\r\n\
            asdf1234\r\n\
            --boundary--"
        );

        assert_boundary(&mut finder, false);
        assert_stream_eq(&mut finder, &[b"asdf1234"]);
        assert_boundary(&mut finder, true);
    }
}

