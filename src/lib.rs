// Copyright 2017 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//! Client- and server-side abstractions for HTTP `multipart/form-data` requests.
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
//!
//! * `iron`: Enable integration with the [Iron](http://ironframework.io) web application
//! framework. See the [`server::iron`](server/iron/index.html) module for more information.
//!
//! * `tiny_http`: Enable integration with the [`tiny_http`](https://github.com/frewsxcv/tiny-http)
//! crate. See the [`server::tiny_http`](server/tiny_http/index.html) module for more information.
#![warn(missing_docs)]
#[macro_use] extern crate log;
extern crate env_logger;

extern crate display_bytes;

#[macro_use]
extern crate futures;

extern crate mime;
extern crate mime_guess;
extern crate rand;

extern crate tempdir;

#[cfg(feature = "hyper")]
extern crate hyper;

use rand::Rng;

use std::rc::Rc;
use std::sync::Arc;

// FIXME: after server prototype is working
//#[cfg(feature = "client")]
//pub mod client;

#[cfg(feature = "server")]
pub mod server;

mod helpers;

/*#[cfg(all(test, feature = "client", feature = "server"))]
mod local_test;
*/
fn random_alphanumeric(len: usize) -> String {
    rand::thread_rng().gen_ascii_chars().take(len).collect()
}
