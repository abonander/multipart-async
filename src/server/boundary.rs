// Copyright 2016 `multipart` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
extern crate twoway;

use self::twoway;
use self::memchr;

use std::cmp;
use std::borrow::Borrow;

use std::io;
use std::io::prelude::*;

use super::{Result, Error};
use super::Error::*;

/// A struct implementing `Read` and `BufRead` that will yield bytes until it sees a given sequence.
#[derive(Debug)]
pub struct BoundaryFinder<S: Stream> {
    stream: S,
    state: BoundaryState<S::Item>,
    boundary: Vec<u8>
}

impl<S> BoundaryFinder<S> {
    #[doc(hidden)]
    pub fn new<B: Into<Vec<u8>>>(stream: S, boundary: B) -> BoundaryFinder<S> {
        BoundaryFinder {
            stream: stream,
            state: BoundaryState::Fresh,
            boundary: boundary.into(),
        }
    }

    pub fn find_boundary(&mut self) -> &[u8] {

    }


    #[doc(hidden)]
    pub fn consume_boundary(&mut self) -> Result<()> {
        if self.at_end {
            return Err(Error::AtEnd);
        }

        while !self.boundary_read {
            let buf_len = self.find_boundary().len();

            if buf_len == 0 {
                return Err(Error::ReadMore);
            }

            self.buf.consume(buf_len);
        }

        self.buf.consume(self.search_idx + self.boundary.len());

        self.search_idx = 0;
        self.boundary_read = false;
 
        Ok(())
    }
}

enum BoundaryState<B> {
    /// Waiting for next boundary
    Watching,
    Partial(B),
    /// The second half after a partial boundary
    AfterPartial(B),
    Read,
    End
}

/// Check if `needle` is cut off at the end of `haystack`, and if so, its index
fn partial_rmatch(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if haystack.is_empty() || needle.is_empty() { return None; }
    if haystack.len() < needle.len() { return None; }

    let trim_start = haystack.len() - (needle.len() - 1);

    let idx = try_opt!(twoway::find_bytes(&haystack[trim_start..], &needle[..1])) + trim_start;

    // If the rest of `haystack` matches `needle`, then we have our partial match
    if haystack[idx..].iter().zip(needle).all(|l, r| l == r) {
        Some(idx)
    } else {
        None
    }
}

#[cfg(test)]
mod test {
    use super::BoundaryFinder;

    use std::io;
    use std::io::prelude::*;

    const BOUNDARY: &'static str = "\r\n--boundary";
    const TEST_VAL: &'static str = "\r\n--boundary\r
dashed-value-1\r
--boundary\r
dashed-value-2\r
--boundary--"; 
        
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
