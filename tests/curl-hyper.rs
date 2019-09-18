//! Test sending a request from cURL to a Hyper endpoint
use std::error::Error;
use std::net::SocketAddr;

use http::{Method, HeaderMap, StatusCode, Request};
use hyper::{Response, Server, Body};
use hyper::service::{make_service_fn, service_fn};

use futures::{future, FutureExt, StreamExt, TryStreamExt};

use multipart_async::server::Multipart;
use curl::easy::{Handler, Easy2, WriteError, Form, ReadError};
use std::thread;
use std::ops::Range;

struct TestHandler;

impl Handler for TestHandler {}

#[tokio::test]
async fn test_hyper_with_curl() {
    // set 0 for the port to have the OS pick one
    let addr: SocketAddr = ([127, 0, 0, 1], 0).into();

    let builder = Server::bind(&addr);

    let (tx, mut rx) = futures::channel::mpsc::channel::<()>(0);

    // And a MakeService to handle each connection...
    let make_service = make_service_fn(move |_| {
        let tx = tx.clone();

        future::ok::<_, hyper::Error>(service_fn(move |req: Request<Body>| {
            let mut tx = tx.clone();

            async move {
                assert_eq!(req.uri().path(), "/multipart-upload");

                let mut multipart = Multipart::try_from_request(req).unwrap();
                let mut normal_done = false;
                let mut text_done = false;
                let mut binary_done = false;

                while let Some(mut field) = multipart.next_field().await.unwrap() {
                    match &field.headers.name[..] {
                        "normal" => {
                            assert!(!normal_done);
                            normal_done = true;

                            assert!(field.headers.is_text());
                            assert_eq!(field.headers.filename, None);
                            assert_eq!(field.headers.content_type, None);
                            assert_eq!(field.headers.ext_headers, HeaderMap::new());
                            assert_eq!(field.data.read_to_string().await.unwrap(), "Hello, world!");
                        }
                        "text-field" => {
                            assert!(!text_done);
                            text_done = true;

                            assert_eq!(field.headers.filename, Some("text-file.txt".to_string()));
                            assert!(field.headers.is_text());
                            assert_eq!(field.headers.content_type, Some(mime::TEXT_PLAIN));
                            assert_eq!(field.headers.ext_headers, HeaderMap::new());
                            assert_eq!(field.data.read_to_string().await.unwrap(),
                                       "Hello, world from a text file!");
                        }
                        "binary-field" => {
                            assert!(!binary_done);
                            binary_done = true;

                            assert_eq!(field.headers.filename, Some("binary-file.bin".to_string()));
                            assert!(!field.headers.is_text());
                            assert_eq!(field.headers.content_type, Some(mime::APPLICATION_OCTET_STREAM));
                            assert_eq!(field.headers.ext_headers, HeaderMap::new());

                            let mut data = 0u8..=255;

                            if let Some(chunk) = field.data.try_next().await.unwrap() {
                                let mut chunk_data = chunk.iter();
                                assert!(chunk_data.by_ref().zip(&mut data).all(|(&l, r)| l == r));
                                assert_eq!(chunk_data.next(), None);
                            }

                            assert_eq!(data.next(), None);
                        }
                        unknown => panic!("unexpected field {:?}", unknown),
                    }
                }

                assert!(normal_done);
                assert!(text_done);
                assert!(binary_done);

                tx.try_send(()).unwrap();

                Response::builder().status(StatusCode::OK).body(Body::empty())
            }
        }))
    });

    let server = builder.serve(make_service);

    let bind_addr = server.local_addr();

    thread::spawn(move || {
        let mut form = Form::new();
        form.part("normal").contents("Hello, world!".as_bytes()).add().unwrap();
        form.part("text-field")
            .buffer("text-file.txt", "Hello, world from a text file!".into())
            .content_type("text/plain")
            .add().unwrap();

        let contents = (0..=255).collect::<Vec<u8>>();

        form.part("binary-field")
            .buffer("binary-file.bin", contents)
            .content_type("application/octet-stream")
            .add().unwrap();

        let mut easy = Easy2::new(TestHandler);
        easy.url(&format!("http://{}/multipart-upload", bind_addr)).unwrap();
        easy.post(true).unwrap();
        easy.httppost(form).unwrap();

        easy.perform().unwrap();
    });

    server.with_graceful_shutdown(rx.next().map(|res| ())).await.unwrap()
}
