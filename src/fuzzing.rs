//! ### Note: not stable APIS
//! The items exported in this module are not considered part of this crate's public API
//! and may receive breaking changes in semver-compatible versions.
use futures::{stream, StreamExt, TryStream};
use futures::Poll::*;
use futures_test::stream::StreamTestExt;
use futures_test::task::noop_context;

use std::cmp;

pub use crate::server::fuzzing::*;

pub use crate::StringError;

/// Deterministically chunk test data so the fuzzer can discover new code paths
pub fn chunk_test_data<'d>(mut data: &'d [u8]) -> impl TryStream<Ok = &'d [u8], Error = StringError> + 'd{
    stream::iter(data.chunks(BOUNDARY.len() - 1))
        .map(Ok)
        .interleave_pending()
}

pub fn fuzz_boundary_finder(test_data: &[u8]) {
    let finder = BoundaryFinder::new(chunk_test_data(test_data), BOUNDARY);
    pin_mut!(finder);

    let mut cx = noop_context();

    loop {
        match finder.as_mut().consume_boundary(&mut cx) {
            Ready(Ok(false)) | Ready(Err(_)) => return,
            Ready(Ok(true)) => (),
            Pending => continue,
        }

        loop {
            match finder.as_mut().body_chunk(&mut cx) {
                Ready(Some(Ok(chunk))) => {
                    assert_ne!(chunk, &[]);
                    assert_eq!(twoway::find_bytes(chunk, BOUNDARY.as_bytes()), None)
                },
                Pending => (),
                Ready(None) | Ready(Some(Err(_))) => return,
            }
        }
    }
}


pub const BOUNDARY: &str = "--boundary";

#[test]
fn test_fuzz_boundary_finder() {
    fuzz_boundary_finder(b"--boundary\r\n");
}
