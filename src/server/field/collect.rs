use futures::Poll;
use futures::Async::*;

use std::str;

use server::boundary::BoundaryFinder;
use server::{BodyChunk, StreamError};

use helpers::*;

#[derive(Default, Debug)]
pub struct CollectStr {
    accum: String,
    limit: Option<u64>,
}

impl CollectStr {
    pub fn collect<S: Stream>(&mut self, stream: &mut BoundaryFinder<S>) -> Poll<String, S::Error>
    where S::Item: BodyChunk, S::Item: StreamError {

        loop {
            let chunk = match try_ready!(stream.body_chunk()) {
                Some(val) => val,
                N => break,
            };

            if let Some(limit) = self.limit {
                if (self.accum.len() as u64).saturating_add(chunk.len()) {
                    stream.push_chunk(chunk);
                }
            }

            let split_idx = match str::from_utf8(chunk.as_slice()) {
                Ok(s) => { self.accum.push_str(s); continue },
                Err(e) => match e.error_len() {
                    Some(_) => return error(e),
                    None => e.valid_up_to(),
                },
            };

            let (valid, invalid) = chunk.split_at(split_idx);

            let [first, second] = try_opt_ready!(stream.another_chunk(invalid));


        }

        ready(replace_default(&mut self.accum))
    }

}

