use std::future::Future;
use std::task::Poll::*;
use std::thread;
use std::time::Duration;

use futures_core::stream::{TryStream, Stream};
use futures_core::task::Context;

use futures_test::stream::StreamTestExt;
use futures_test::task::noop_context;

use futures_util::stream::{self, StreamExt};

use crate::StringError;

pub const BOUNDARY: &str = "--boundary";

pub const TEST_SINGLE_FIELD: &[&[u8]] = &[
    b"--boundary\r", b"\n",
    b"Content-Disposition:",
    b" form-data; name=",
    b"\"foo\"",
    b"\r\n\r\n",
    b"field data",
    b"\r", b"\n--boundary--"
];

pub fn mock_stream<'d>(test_data: &'d [&'d [u8]]) -> impl Stream<Item = Result<&'d [u8], StringError>> + 'd {
    stream::iter(test_data.iter().cloned()).map(Ok).interleave_pending()
}

macro_rules! until_ready(
    (|$cx:ident| $expr:expr) => {{
        use std::task::Poll::*;
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
        use std::task::Poll::*;
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
        use std::task::Poll::*;
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

pub fn run_future_hot<F>(f: F) -> F::Output where F: Future {
    pin_mut!(f);
    until_ready!(|cx| f.as_mut().poll(cx))
}
