// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use futures::{Async, Stream};

use std::error::Error;
use std::io;
use std::mem;

pub use display_bytes::display_bytes as show_bytes;

pub use futures::Poll;

pub type PollOpt<T, E> = Poll<Option<T>, E>;

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

pub fn replace_default<T: Default>(dest: &mut T) -> T {
    mem::replace(dest, T::default())
}
