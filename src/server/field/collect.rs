use futures::Stream;
use futures::Async::*;

use std::str;

use server::boundary::BoundaryFinder;
use server::{BodyChunk, StreamError};

use helpers::*;

#[derive(Default, Debug)]
pub struct CollectStr {
    accum: String,
    pub limit: Option<usize>,
}

impl CollectStr {
    pub fn collect<S: Stream>(&mut self, stream: &mut BoundaryFinder<S>) -> Poll<String, S::Error>
    where S::Item: BodyChunk, S::Error: StreamError {
        loop {
            let chunk = match try_ready!(stream.body_chunk()) {
                Some(val) => val,
                N => break,
            };

            if let Some(limit) = self.limit {
                // This also catches capacity overflows but only when a limit is set
                if self.accum.len().saturating_add(chunk.len()) > limit {
                    stream.push_chunk(chunk);
                    return Err(StreamError::from_string(format!("String field exceeded limit of \
                                                                  {} bytes", limit)));
                }
            }

            // Try to convert the chunk to UTF-8 and append it to the accumulator
            let split_idx = match str::from_utf8(chunk.as_slice()) {
                Ok(s) => { self.accum.push_str(s); continue },
                Err(e) => match e.error_len() {
                    // a non-null `error_len` means there was an invalid byte sequence
                    Some(_) => return error(e),
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

            let mut buf = [0u8; 4];

            // first.len() will be between 1 and 4 as guaranteed by `FromUtf8Error::valid_up_to()`
            buf[..first.len()].copy_from_slice(first.as_slice());
            buf[first.len()..].copy_from_slice(&second.as_slice()[.. needed_len]);

            let split_idx = match str::from_utf8(&buf) {
                Ok(s) => { self.accum.push_str(s); needed_len },
                Err(e) => match e.error_len() {
                    Some(_) => return error(e),
                    None => e.valid_up_to(),
                }
            };

            let (_, rem) = second.split_at(split_idx);

            if !rem.is_empty() {
                stream.push_chunk(rem);
            }
        }

        ready(replace_default(&mut self.accum))
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
