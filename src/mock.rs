use futures::channel::mpsc;
use futures::executor::block_on_stream;
use futures::stream::{self, Stream, StreamExt};
use futures::Poll;
use std::thread;
use std::time::Duration;
use StringError;

lazy_static! {
    pub(crate) static ref SENDER: mpsc::Sender<()> = {
        let (tx, rx) = mpsc::channel(0);
        thread::spawn(move || {
            for _ in block_on_stream(rx) {
                thread::sleep(Duration::from_millis(10));
            }
        });
        tx
    };
}

pub(crate) trait IntoResult {
    fn into_result(self) -> Result<&'static [u8], StringError>;
}

impl IntoResult for &'static [u8] {
    fn into_result(self) -> Result<&'static [u8], StringError> {
        Ok(self)
    }
}

impl IntoResult for Result<&'static [u8], StringError> {
    fn into_result(self) -> Self {
        self
    }
}

impl IntoResult for &'static str {
    fn into_result(self) -> Result<&'static [u8], StringError> {
        Err(StringError(self.into()))
    }
}

/// Get a stream which yields the `$elem` series punctuated by nondeterministic `Pending` values
macro_rules! mock_stream {
    ($($elem:expr),*) => {{
        use futures::stream;
        use futures::Poll;
        use mock::{IntoResult, SENDER};
        use StringError;

        let mut tx = SENDER.clone();
        let mut iter = vec![$($elem.into_result()),*].into_iter();
        stream::poll_fn::<Result<&'static [u8], StringError>, _>(move |cx| {
            ready!(tx.poll_ready(cx)).unwrap();
            Poll::Ready(iter.next())
        })
    }};
}
