// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use std::borrow::Cow;
use std::fmt;
use std::mem;
use std::str::Utf8Error;

use crate::StreamError;

pub use futures_core::*;
use std::task::Poll::{self, *};

pub type PollOpt<T, E> = Poll<Option<Result<T, E>>>;

pub fn ready_ok<R, T, E>(val: T) -> Poll<R>
where
    R: From<Result<T, E>>,
{
    Poll::Ready(Ok(val).into())
}

pub fn error<T, E: Into<Cow<'static, str>>, E_: StreamError, R>(e: E) -> R
where
    R: From<Result<T, E_>>,
{
    match e.into() {
        Cow::Owned(string) => Err(E_::from_string(string).into()).into(),
        Cow::Borrowed(str) => Err(E_::from_str(str)).into(),
    }
}

pub fn ready_err<T, E: Into<Cow<'static, str>>, E_: StreamError, R>(e: E) -> Poll<R>
where
    R: From<Result<T, E_>>,
{
    match e.into() {
        Cow::Owned(string) => Poll::Ready(Err(E_::from_string(string).into()).into()),
        Cow::Borrowed(str) => Poll::Ready(Err(E_::from_str(str)).into()),
    }
}

pub fn utf8_err<T, E: StreamError>(e: Utf8Error) -> Result<T, E> {
    Err(E::from_utf8(e))
}

pub fn replace_default<T: Default>(dest: &mut T) -> T {
    mem::replace(dest, T::default())
}

pub fn show_bytes(bytes: &[u8]) -> impl fmt::Display + '_ {
    display_bytes::HEX_UTF8.clone()
        .escape_control(true)
        .min_str_len(1).display_bytes(bytes)
}
