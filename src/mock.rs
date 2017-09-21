use std::borrow::Cow;

use helpers::*;

#[macro_export]
macro_rules! mock_stream {
    ($($args:tt)*) => ({
        struct MockStream(u32);

        #[allow(unused_comparisons)]
        impl $crate::futures::Stream for MockStream {
            type Item = $crate::std::borrow::Cow<'static, [u8]>;
            type Error = $crate::mock::StringError;

            fn poll(&mut self) -> $crate::futures::Poll<Option<Self::Item>, Self::Error> {
                let state = self.0;
                self.0 += 1;

                stream_items!(@(state, 0) $($args)*);

                panic!("MockStream::poll() called after returning `None`, state: {}", state);
            }
        }

        MockStream(0)
    });
}

#[doc(hidden)]
#[macro_export]
macro_rules! stream_items (
    (@($state:ident, $val:expr) $ex:expr) => (
        stream_items!(@($state, $val) $ex;)
    );
    (@($state:ident, $val:expr) $ex:expr, $repeat:expr) => (
        stream_items!(@($state, $val) $ex, $repeat;)
    );
    (@($state:ident, $val:expr) $ex:expr; $($rest:tt)*) => (
        if $state == $val { return $crate::mock::into_poll($ex) }
        stream_items!(@($state, $val + 1) $($rest)*);
    );
    (@($state:ident, $val:expr) $ex:expr, $repeat:expr; $($rest:tt)*) => (
        if $state >= $val && $state <= $val + $repeat {
            return $crate::mock::into_poll($ex)
        }

        stream_items!(@($state, $val + $repeat + 1) $($rest)*);
    );
    (@($state:ident, $val:expr)) => (
        if $state == $val { return $crate::helpers::ready(None) }
    );
);

#[doc(hidden)]
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

/// An adaptor trait to make `mock_stream!()` easier to use,
/// shouldn't be necessary for users to be aware of it.
#[doc(hidden)]
pub trait IntoPoll {
    fn into_poll(self) -> Poll<Option<Cow<'static, [u8]>>, StringError>;
}

impl<T: AsRef<[u8]> + ?Sized> IntoPoll for &'static T {
    fn into_poll(self) -> PollOpt<Cow<'static, [u8]>, StringError> {
        ready(Some(self.as_ref().into()))
    }
}

impl IntoPoll for Vec<u8> {
    fn into_poll(self) -> PollOpt<Cow<'static, [u8]>, StringError> {
        ready(Some(self.into()))
    }
}

impl IntoPoll for Option<Cow<'static, [u8]>> {
    fn into_poll(self) -> Poll<Option<Cow<'static, [u8]>>, StringError> {
        ready(self)
    }
}

impl<E: Into<String>> IntoPoll for PollOpt<Cow<'static, [u8]>, E> {
    fn into_poll(self) -> Poll<Option<Cow<'static, [u8]>>, StringError> {
        self.map_err(|s| StringError(s.into()))
    }
}

#[doc(hidden)]
pub fn into_poll<T: IntoPoll>(from: T) -> Poll<Option<Cow<'static, [u8]>>, StringError> {
    from.into_poll()
}

#[cfg(test)]
mod test {
    use std::borrow::Cow;

    use helpers::*;

    use super::into_poll;

    #[test]
    fn test_into_poll() {
        assert_eq!(
            Ok(Async::Ready(Some(Cow::Borrowed(&b"Hello, world!"[..])))),
            into_poll("Hello, world!")
        );
    }

    #[test]
    fn test_empty_mock() {
        assert_eq!(mock_stream!().poll(), into_poll(None));
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
        let mut stream = mock_stream!("Hello, world!");
        assert_eq!(stream.poll(), into_poll("Hello, world!"));
        assert_eq!(stream.poll(), ready(None));
    }

    #[test]
    fn test_repeat_once() {
        let mut stream = mock_stream!("Hello, world!", 1);
        assert_eq!(stream.poll(), into_poll("Hello, world!"));
        assert_eq!(stream.poll(), ready(Some(b"Hello, world!".as_ref().into())));
        assert_eq!(stream.poll(), ready(None));
    }

    #[test]
    fn test_two_items() {
        let mut stream = mock_stream!("Hello, world!"; "Hello, also!");
        assert_eq!(stream.poll(), into_poll("Hello, world!"));
        assert_eq!(stream.poll(), into_poll("Hello, also!"));
        assert_eq!(stream.poll(), ready(None));
    }

    #[test]
    fn test_two_items_one_repeat() {
        let mut stream = mock_stream!("Hello, world!", 1; "Hello, also!");
        assert_eq!(stream.poll(), into_poll("Hello, world!"));
        assert_eq!(stream.poll(), into_poll("Hello, world!"));
        assert_eq!(stream.poll(), into_poll("Hello, also!"));
        assert_eq!(stream.poll(), ready(None));
    }
}
