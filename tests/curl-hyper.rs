//! Test sending a request from cURL to a Hyper endpoint
#![feature(async_await)]

use std::error::Error;
use std::net::SocketAddr;

use http::Method;
use hyper::{Response, Server};
use hyper::service::{make_service_fn, service_fn};

use multipart_async::server::Multipart;

fn main() -> Result<(), Box<dyn Error>>{
    // set 0 for the port to have the OS pick one
    let addr: SocketAddr = ([127, 0, 0, 1], 0).into();

    let builder = Server::try_bind(&addr)?;

    // And a MakeService to handle each connection...
    let make_service = make_service_fn(|_| async {
        Ok::<_, hyper::Error>(service_fn(|req| async {
            assert_eq!(req.uri().path(), "/multipart-upload");

            let mut multipart = Multipart::try_from_request(req).unwrap();

            if let Some(field) = multipart.next_field().await.unwrap()? {
                if field.headers.name == "normal" {
                    let data = field.data.
                }
            }
        }))
    });

}
