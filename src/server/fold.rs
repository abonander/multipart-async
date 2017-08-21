use futures::{Poll, Stream};

use field::Field;

use super::{BodyChunk, Multipart, StreamError};

pub struct FoldFields<F, R, S> {
    folder: F,
    state: R,
    multipart: Multipart<S>
}

impl<F, R, S: Stream> Future for FoldFields<F, R, S> where S::Item: BodyChunk, S::Error: StreamError,
                                                           F: FnMut(&mut R, Field<S>) -> Poll<(), S::Error> {
    type Item = R;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<R, S::Error> {

    }
}
