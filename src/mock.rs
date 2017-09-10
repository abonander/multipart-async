use futures::{Async, Stream};

use helpers::*;

#[macro_export]
macro_rules! mock_stream {
    ($ex:expr, $repeat:expr) => (
        mock_stream!(($repeat) { 0 .. $repeat => $ex, })
    );
    ($ex:expr) => (
        mock_stream!((1) { 0 => $ex, })
    );
    /*($ex:expr, $repeat:expr; $($rest:tt)+) => (
        mock_stream!(($repeat) $($rest)* { 0 .. $repeat => $ex, })
    );
    ($ex:expr; $($rest:tt)+) => (
        mock_stream!((1) $($rest)* { 0 => $ex, })
    );
    (($i:expr) $ex:expr, $repeat:expr; $($rest:tt)* {$($branches:tt)* }) => (
        mock_stream!(($i + $repeat) $($rest)* { $($branches)* $i .. $i + $repeat => $ex })
    );
    (($i:expr) $ex:expr; $($rest:tt)* {$($branches:tt)* }) => (
        mock_stream!(($i + 1) $($rest)* { $($branches)* $i => $ex })
    );*/
    (($end:expr) {$($branches:tt)*}) => ({
        struct MockStream(u32);

        impl $crate::futures::Stream for MockStream {
            type Item = $crate::std::borrow::Cow<'static, [u8]>;
            type Error = $crate::mock::StringError;

            fn poll(&mut self) -> $crate::futures::Poll<Option<Self::Item>, Self::Error> {
                let state = self.0;
                self.0 += 1;

                match state {
                    $($branches)*
                    $end => Ok($crate::futures::Async::Ready(None)),
                    _ => panic!("MockStream::poll() called after returning `None`"),
                }
            }
        }

        MockStream(0)
    });
    () => (
        mock_stream!((0) {})
    )
}

#[derive(Debug, Eq, PartialEq)]
pub struct StringError(String);

impl PartialEq<String> for StringError {
    fn eq(&self, other: &String) -> bool {
        *self == **other
    }
}

impl PartialEq<str> for StringError {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

#[test]
fn test_empty_mock() {
    assert_eq!(mock_stream!().poll(), Ok(Async::Ready(None)));
}

#[test]
#[should_panic]
fn test_extra_poll() {
    let mut stream = mock_stream!();
    let _ = stream.poll();
    let _ = stream.poll();
}

#[test]
fn test_yield_once() {
    let mut stream = mock_stream!(ready(b"Hello, world!"));
    assert_eq!(stream.poll(), Ok(Async::Ready(b"Hello, world!".into())))
}

pub trait IntoPoll {
    fn into_poll(self) -> Poll<Option<Cow<'static, [u8]>>, StringError>;
}

impl
