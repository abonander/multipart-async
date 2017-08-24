// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
extern crate twoway;

use futures::{Async, Poll, Stream};

use std::cmp;
use std::borrow::{Borrow, Cow};
use std::error::Error;
use std::fmt;
use std::io;
use std::mem;

use super::BodyChunk;

use self::State::*;

use helpers::*;

pub type PollOpt<T, E> = Poll<Option<T>, E>;

/// A struct implementing `Read` and `BufRead` that will yield bytes until it sees a given sequence.
// #[derive(Debug)]
pub struct BoundaryFinder<S: Stream> {
    stream: S,
    state: State<S::Item>,
    boundary: Box<[u8]>,
    pushed: Option<S::Item>,
}

impl<S: Stream> BoundaryFinder<S> {
    pub fn new<B: Into<Vec<u8>>>(stream: S, boundary: B) -> BoundaryFinder<S> {
        BoundaryFinder {
            stream: stream,
            state: State::Watching,
            boundary: boundary.into().into_boxed_slice(),
            pushed: None,
        }
    }
}

impl<S: Stream> BoundaryFinder<S> where S::Item: BodyChunk, S::Error: From<io::Error> {

    pub fn push_chunk(&mut self, chunk: S::Item) {
        debug_assert!(twoway::find_bytes(chunk.as_slice(), &self.boundary).is_none(),
                      "Pushed chunk contains boundary: {}", show_bytes(chunk.as_slice()));

        debug_assert!(self.pushed.is_none(),
                      "Pushing a chunk when there already was one: {} Pushed: {}",
                      show_bytes(self.pushed.take().unwrap().as_slice()), show_bytes(chunk.as_slice()));

        if chunk.is_empty() {
            debug!("BoundaryFinder::push_chunk() called with empty chunk");
            return;
        }

        self.pushed = Some(chunk);
    }

    /// Try to poll for another chunk; if successful, return both of them, otherwise push the first
    /// chunk back.
    pub fn another_chunk(&mut self, first: S::Item) -> PollOpt<(S::Item, S::Item), S::Error> {
        match self.body_chunk() {
            Ok(Async::Ready(Some(second))) => ready(Some((first, second))),
            Ok(Async::Ready(None)) => Ok(Async::Ready(None)),
            Ok(Async::NotReady) => { self.push_chunk(first); Ok(Async::NotReady) }
            Err(e) => { self.push_chunk(first); Err(e) },
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
            trace!("body_chunk() loop state: {:?} pushed: {:?}", self.state,
                   self.pushed.as_ref().map(|c| c.as_slice()));

            if let Some(pushed) = self.pushed.take() {
                return ready(pushed);
            }

            match self.state {
                Boundary(_) | BoundarySplit(_, _) | End => return ready(None),
                _ => ()
            }

            match mem::replace(&mut self.state, Watching) {
                Watching => {
                    let chunk = try_ready_opt!(self.stream.poll());

                    // For sanity
                    if chunk.is_empty() { return ready(chunk); }

                    return self.check_chunk(chunk);
                },
                Remainder(rem) => return self.check_chunk(rem),
                Partial(partial, res) => {
                    let chunk = try_ready_opt!(self.stream.poll(); Partial(partial, res));
                    let needed_len = (self.boundary_size(res.incl_crlf)).saturating_sub(partial.len());

                    if needed_len > chunk.len() {
                        // hopefully rare
                        return error("chunk too short to verify multipart boundary");
                    }

                    if self.check_boundary_split(&partial.as_slice()[res.boundary_start()..], chunk.as_slice()) {
                        let (ret, first) = partial.split_at(res.boundary_start());
                        self.state = BoundarySplit(first, chunk);

                        if !ret.is_empty() {
                            return ready(ret);
                        } else {
                            // Don't return an empty chunk at the end
                            return ready(None);
                        }
                    }

                    self.state = Remainder(chunk);
                    return ready(partial);
                },
                state => unreachable!("invalid state: {:?}", state),
            }
        }
    }

    fn check_chunk(&mut self, chunk: S::Item) -> PollOpt<S::Item, S::Error> {
        trace!("check chunk: {}", show_bytes(chunk.as_slice()));

        if let Some(res) = self.find_boundary(&chunk) {
            debug!("boundary found: {:?}", res);

            let len = self.boundary_size(res.incl_crlf);

            if chunk.len() < res.idx + len {
                // Either partial boundary, or boundary but not the two bytes after it
                self.state = Partial(chunk, res)
            } else {
                let (ret, bnd) = chunk.split_at(res.idx);

                let bnd = if res.incl_crlf {
                    // cut off the preceding CRLF
                    bnd.split_at(2).1
                } else {
                    bnd
                };

                self.state = Boundary(bnd);

                if !ret.is_empty() {
                    return ready(ret);
                } else {
                    return ready(None);
                }
            };

            return not_ready();
        } else {
            ready(chunk)
        }
    }

    fn find_boundary(&self, chunk: &S::Item) -> Option<SearchResult> {
        twoway::find_bytes(chunk.as_slice(), &self.boundary)
            .map(|idx| check_crlf(chunk.as_slice(), idx))
            .or_else(|| self.partial_find_boundary(chunk))
    }

    fn partial_find_boundary(&self, chunk: &S::Item) -> Option<SearchResult> {
        let chunk = chunk.as_slice();
        let len = chunk.len();

        partial_rmatch(chunk, &self.boundary)
            .map(|idx| check_crlf(chunk, idx))
            .or_else(||
                // EDGE CASE: the bytes of the newline before the boundary are at the end
                // of the chunk
                if len > 2 && &chunk[len - 2 ..] == &*b"\r\n" {
                    Some(SearchResult {
                        idx: len - 2,
                        incl_crlf: true,
                    })
                } else if len > 1 && chunk[len - 1] == b'\r' {
                    Some(SearchResult {
                        idx: len - 1,
                        incl_crlf: true
                    })
                } else {
                    None
                }
            )
    }

    fn maybe_boundary(&self, bytes: &[u8]) -> bool {
        (bytes.len() >= 2 && self.boundary.starts_with(&bytes[2..]))
            || self.boundary.starts_with(bytes)
    }

    fn check_boundary(&self, bytes: &[u8]) -> bool {
        (bytes.len() >= 2 && bytes[2..].starts_with(&self.boundary))
            || bytes.starts_with(&self.boundary)
    }

    fn check_boundary_split(&self, first: &[u8], second: &[u8]) -> bool {
        let check_len = self.boundary.len() - first.len();

        second.len() >= check_len && first.iter().chain(&second[..check_len])
            .eq(self.boundary.iter())
    }

    pub fn consume_boundary(&mut self) -> Poll<bool, S::Error> {
        debug!("consuming boundary");

        while try_ready!(self.body_chunk()).is_some() {}

        match mem::replace(&mut self.state, Watching) {
            Boundary(bnd) => self.confirm_boundary(bnd),
            BoundarySplit(first, second) => self.confirm_boundary_split(first, second),
            state => unreachable!("invalid state: {:?}", state),
        }
    }

    fn confirm_boundary(&mut self, boundary: S::Item) -> Poll<bool, S::Error> {
        if boundary.len() < self.boundary_size(false) {
            return error(format!("boundary sequence too short: {}",
                                 show_bytes(boundary.as_slice())));
        }

        let (boundary, rem) = boundary.split_at(self.boundary_size(false));
        let boundary = boundary.as_slice();

        trace!("confirming boundary: {}", show_bytes(boundary));

        debug_assert!(!boundary.starts_with(b"\r\n"),
                      "leading CRLF should have been trimmed from boundary: {}",
                      show_bytes(boundary));

        debug_assert!(self.check_boundary(boundary),
                      "invalid boundary previous confirmed as valid: {}",
                      show_bytes(boundary));

        self.state = if !rem.is_empty() { Remainder(rem) } else { Watching };

        trace!("boundary found: {}", show_bytes(boundary));

        let len = boundary.len();

        let is_end = check_last_two(boundary);

        debug!("is_end: {:?}", is_end);

        if is_end { self.state = End; }

        ready(is_end)
    }

    fn confirm_boundary_split(&mut self, first: S::Item, second: S::Item) -> Poll<bool, S::Error> {
        let first = first.as_slice();
        let check_len = self.boundary_size(false) - first.len();

        if second.len() < check_len {
            return error(format!("split boundary sequence too short: ({}, {})",
                                 show_bytes(first),
                                 show_bytes(second.as_slice())));
        }

        let (second, rem) = second.split_at(check_len);
        let second = second.as_slice();

        self.state = Remainder(rem);

        debug_assert!(!first.starts_with(b"\r\n"),
                      "leading CRLF should have been trimmed from first boundary section: {}",
                      show_bytes(first));

        debug_assert!(self.check_boundary_split(first, second),
                      "invalid split boundary previous confirmed as valid: ({}, {})",
                      show_bytes(first), show_bytes(second));

        let is_end = check_last_two(second);

        if is_end { self.state = End; }

        ready(is_end)
    }

    /// The necessary size to verify a boundary, including the potential CRLF before, and the
    /// CRLF / "--" afterward
    fn boundary_size(&self, incl_crlf: bool) -> usize {
        self.boundary.len() + if incl_crlf { 4 } else { 2 }
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
    /// Partial boundary
    Partial(B, SearchResult),
    Boundary(B),
    BoundarySplit(B, B),
    /// The remains of a chunk after processing
    Remainder(B),
    End,
}

impl<B: BodyChunk> fmt::Debug for State<B> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use std::fmt::Write;
        use self::State::*;

        match *self {
            Watching => f.write_str("State::Watching"),
            Partial(ref bnd, res) => write!(f, "State::Partial({}, {:?})", show_bytes(bnd.as_slice()), res),
            Boundary(ref bnd) => write!(f, "State::Boundary({})", show_bytes(bnd.as_slice())),
            BoundarySplit(ref first, ref second) => write!(f, "State::BoundarySplit({}, {})",
                                                           show_bytes(first.as_slice()),
                                                           show_bytes(second.as_slice())),
            Remainder(ref rem) => write!(f, "State::Remainder({})", show_bytes(rem.as_slice())),
            End => f.write_str("State::End"),
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct SearchResult {
    idx: usize,
    incl_crlf: bool,
}

impl SearchResult {
    fn boundary_start(&self) -> usize {
        if self.incl_crlf { self.idx + 2 } else { self.idx }
    }
}

/// If there's a CRLF before the boundary, we want to back up to make sure we don't yield a newline
/// that the client doesn't expect
fn check_crlf(chunk: &[u8], mut idx: usize) -> SearchResult {
    let mut incl_crlf = false;
    if idx > 2 && chunk[idx - 2 .. idx] == *b"\r\n" {
        incl_crlf = true;
        idx -= 2;
    }

    SearchResult {
        idx: idx,
        incl_crlf: incl_crlf
    }
}

fn check_last_two(boundary: &[u8]) -> bool {
    let len = boundary.len();

    let is_end = boundary.ends_with(b"--");

    if !is_end && !boundary.ends_with(b"\r\n") && boundary.len() > 2 {
        warn!("unexpected bytes after boundary: {:?} ('--': {:?}, '\\r\\n': {:?})",
              &boundary[len - 2 ..], b"--", b"\r\n");
    }

    is_end
}

/// Check if `needle` is cut off at the end of `haystack`, and if so, its index
fn partial_rmatch(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if haystack.is_empty() || needle.is_empty() { return None; }

    // If the haystack is smaller than the needle, we still need to test it
    let trim_start = haystack.len().saturating_sub(needle.len() - 1);

    let idx = try_opt!(twoway::find_bytes(&haystack[trim_start..], &needle[..1])) + trim_start;

    trace!("partial_rmatch found start: {:?}", idx);

    // If the rest of `haystack` matches `needle`, then we have our partial match
    if haystack[idx..].iter().zip(needle).all(|(l, r)| l == r) {
        Some(idx)
    } else {
        None
    }
}

#[cfg(test)]
mod test {
    use super::{BoundaryFinder, ready, not_ready, show_bytes};

    use server::BodyChunk;

    use futures::{Future, Stream, Poll};
    use futures::Async::*;

    use std::fmt::Debug;
    use std::io;
    use std::io::prelude::*;

    const BOUNDARY: &'static str = "--boundary";

    type TestingFinder = BoundaryFinder<MockStream>;

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
        type Item = &'static [u8];
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

            ready(self.stream[curr].0)
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

    fn assert_part(finder: &mut TestingFinder, right: &[&[u8]]) {
        assert_boundary(finder, false);

        let mut right = right.iter();

        loop {
            let left_item = loop {
                match finder.poll().expect("Error from stream") {
                    Ready(item) => break item,
                    _ => (),
                }
            };

            let right_item = right.next();

            match (left_item, right_item) {
                // Option only implements T == T
                (Some(ref left), Some(ref right)) if left == *right => (),
                (None, None) => break,
                (left, right) => panic!("Failed assertion: `{:?} == {:?}`", left, right),
            }
        }
    }

    fn assert_end(finder: &mut TestingFinder) {
        assert_boundary(finder, true)
    }

    fn assert_boundary(finder: &mut TestingFinder, is_end: bool) {
        loop {
            match finder.body_chunk().expect("Error from BoundaryFinder") {
                Ready(Some(chunk)) => panic!("Unexpected chunk from BoundaryFinder: {}", show_bytes(chunk.as_slice())),
                Ready(None) => break,
                NotReady => (),
            }
        }

        loop {
            match finder.consume_boundary().expect("Error from BoundaryFinder") {
                Ready(val) => {
                    assert_eq!(val, is_end, "Found wrong kind of boundary");
                    break;
                },
                _ => (),
            }
        }
    }

    macro_rules! test_request {
        ($testnm:ident {$($chunks:tt)*} [$($part:expr),+]) => (
            #[test]
            fn $testnm() {
                let _ = ::env_logger::init();

                let mut finder = mock_finder!($($chunks)*);

                $(assert_part(&mut finder, &$part);)+
                assert_end(&mut finder);
            }
        )
    }

    test_request! {
        simple
        {
            b"--boundary\r\n\
            asdf1234\r\n\
            --boundary--"
        }
        [
            [b"asdf1234"]
        ]
    }

    test_request! {
        split_repeat_once
        {
            b"--boundary\r\n", 1;
            b"asdf1234\r\n\
            --boundary--"
        }
        [
            [b"asdf1234"]
        ]
    }

    test_request! {
        split_mid_bnd
        {
            b"--boun";
            b"dary\r\n\
            asdf1234\r\n\
            --boundary--"
        }
        [
            [b"asdf1234"]
        ]
    }

    test_request! {
        two_parts
        {
            b"--boundary\r\n\
            asdf1234\r\n\
            --boundary\r\n\
            hjkl5678\r\n\
            --boundary--"
        }
        [[b"asdf1234"], [b"hjkl5678"]]
    }
}

