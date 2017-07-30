// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Poll, Async, Stream};

use std::borrow::Cow;
use std::error::Error;
use std::io;

pub fn ready<R, E, T: Into<R>>(val: T) -> Poll<R, E> {
    Ok(Async::Ready(val.into()))
}

pub fn not_ready<T, E>() -> Poll<T, E> {
    Ok(Async::NotReady)
}

pub fn error<T, E: Into<Box<Error + Send + Sync>>, E_: From<io::Error>>(e: E) -> Result<T, E_> {
    Err(io_error(e).into())
}

pub fn io_error<E: Into<Box<Error + Send + Sync>>>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}

pub fn lossy(bytes: &[u8]) -> Cow<str> {
    String::from_utf8_lossy(bytes)
}

pub struct SomeCell<T> {
    opt: Option<T>,
    err: &'static str,
}

impl<T> SomeCell<T> {
    pub fn new(val: T, err: &'static str) -> Self {
        SomeCell {
            opt: Some(val),
            err: err,
        }
    }

    pub fn try_as_ref(&self) -> io::Result<&T> {
        self.opt.as_ref().ok_or_else(|| io_error(self.err))
    }

    pub fn try_as_mut(&mut self) -> io::Result<&mut T> {
        // Sub-borrow for closure
        let err = self.err;
        self.opt.as_mut().ok_or_else(|| io_error(err))
    }

    pub fn try_take(&mut self) -> io::Result<T> {
        self.opt.take().ok_or_else(|| io_error(self.err))
    }
}


impl<S: Stream> Stream for SomeCell<S> where S::Error: From<io::Error> {
    type Item = S::Item;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        self.try_as_mut()?.poll()
    }
}
