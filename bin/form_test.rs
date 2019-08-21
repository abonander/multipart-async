#[macro_use]
extern crate log;

extern crate env_logger;
extern crate futures;
extern crate hyper;
extern crate multipart_async as multipart;

use futures::future::Either;
use futures::Future;
use futures::Stream;

use hyper::server::Http;
use hyper::{Body, Response};

use multipart::server::{Field, Multipart, MultipartService};

use std::net::SocketAddr;

const FORM: &str = include_str!("test_form.html");

fn main() {
    env_logger::init().unwrap();

    let addr: SocketAddr = "127.0.0.1:8080".parse().expect("invalid socket address");

    Http::new()
        .bind(&addr, || {
            Ok(MultipartService {
                multipart: |(multi, _rest): (Multipart<Body>, _)| {
                    let read_field = |field: Field<Body>| {
                        if field.headers.is_text() {
                            Either::A(field.data.read_text().map(|field| {
                                info!("got text field: {:?}", field);
                            }))
                        } else {
                            info!("got file field: {:?}", field.headers);
                            Either::B(field.data.for_each(eat_ok))
                        }
                    };

                    multi
                        .for_each(read_field)
                        .map(|_| response("success"))
                        .or_else(|err| Ok(response(err.to_string())))
                },
                normal: |_| Ok(response(FORM)),
            })
        })
        .expect("failed to bind socket")
        .run()
        .expect("error running server");
}

fn response<B: Into<Body>>(b: B) -> Response {
    Response::new().with_body(b.into())
}

fn eat_ok<T, E>(_val: T) -> Result<(), E> {
    Ok(())
}
