use futures::Async;

#[macro_export]
macro_rules! mock_stream {
    ($end:pat; {$($branches:tt)*}) => ({
        struct MockStream(u32);

        impl $crate::futures::Stream for MockStream {
            type Item = &'static [u8];
            type Error = $crate::mock::StringError;

            fn poll(&mut self) -> $crate::futures::Poll<Option<Self::Item>, Self::Error> {
                let state = self.0;
                self.0 += 1;

                match state {
                    $($branches)*
                    $end => return $crate::futures::Async::Ready(None),
                    _ => panic!("MockStream::poll() called after returning `None`"),
                }
            }
        }
    });
    () => (
        mock_stream!(0; {})
    )
}

pub struct StringError(String);

#[test]
fn test_empty_mock() {
    assert_eq!(mock_stream!().poll(), Async::Ready(None));
}

#[test]
#[should_panic]
fn test_extra_poll() {
    let mut stream = mock_stream!();
    let _ = stream.poll();
    let _ = stream.poll();
}
