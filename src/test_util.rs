use std::thread;
use std::time::Duration;

use futures::{Poll, TryStream};
use futures::stream::{self, Stream, StreamExt};
use futures::task::Context;

use futures_test::stream::StreamTestExt;
use futures_test::task::noop_context;

use crate::StringError;

pub const BOUNDARY: &str = "--boundary";

pub fn mock_stream<'d>(test_data: &'d [&'d [u8]]) -> impl Stream<Item = Result<&'d [u8], StringError>> + 'd {
    stream::iter(test_data.iter().cloned()).map(Ok).interleave_pending()
}

macro_rules! until_ready(
    (|$cx:ident| $expr:expr) => {{
        use futures::Poll::*;
        let ref mut $cx = futures_test::task::noop_context();
        loop {
            match $expr {
                Ready(val) => break val,
                Pending => (),
            }
        }
    }}
);

macro_rules! ready_assert_eq(
    (|$cx:ident| $expr:expr, $eq:expr) => {{
        use futures::Poll::*;
        let ref mut $cx = futures_test::task::noop_context();
        loop {
            match $expr {
                Ready(val) => {
                    assert_eq!(val, $eq);
                    break;
                },
                Pending => (),
            }
        }
    }}
);

macro_rules! ready_assert(
    (|$cx:ident| $expr:expr) => {{
        use futures::Poll::*;
        let ref mut $cx = futures_test::task::noop_context();
        loop {
            match $expr {
                Ready(val) => {
                    assert!(val);
                    break;
                },
                Pending => (),
            }
        }
    }}
);