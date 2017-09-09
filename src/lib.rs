// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! Client- and server-side abstractions for HTTP `multipart/form-data` requests using asynchronous
//! I/O.
//!
//! Features: 
//! 
//! * `client` (default): Enable the client-side abstractions for multipart requests. If the
//! `hyper` feature is also set, enables integration with the Hyper HTTP client API.
//!
//! * `server` (default): Enable the server-side abstractions for multipart requests. If the
//! `hyper` feature is also set, enables integration with the Hyper HTTP server API.
//!
//! * `hyper` (default): Enable integration with the [Hyper](https://github.com/hyperium/hyper) HTTP library 
//! for client and/or server depending on which other feature flags are set.
#![deny(missing_docs)]
#[macro_use] extern crate log;
extern crate env_logger;

extern crate bytes;
extern crate display_bytes;

#[macro_use]
extern crate futures;

extern crate mime_guess;
extern crate rand;

extern crate tempdir;

#[cfg(feature = "hyper")]
pub extern crate hyper;

pub extern crate mime;

use rand::Rng;

use std::rc::Rc;
use std::sync::Arc;

// FIXME: after server prototype is working
//#[cfg(feature = "client")]
//pub mod client;

#[cfg(feature = "server")]
pub mod server;

mod helpers;
mod mock;

/*#[cfg(all(test, feature = "client", feature = "server"))]
mod local_test;
*/
fn random_alphanumeric(len: usize) -> String {
    rand::thread_rng().gen_ascii_chars().take(len).collect()
}
