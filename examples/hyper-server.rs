use std::net::TcpStream;
use hyper::{Body, Error, Request, Response, Server, StatusCode};
use hyper::rt::{self, Future};
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use multipart_async::server::Multipart;
use httparse::Error::Status;

fn main() {
    let addr = ([127, 0, 0, 1], 3000).into();

    let make_svc = make_service_fn(|socket: &AddrStream| {
        async move {
            Ok::<_, Error>(service_fn(handle_request))
        }
    });

    // Then bind and serve...
    let server = Server::bind(&addr)
        .serve(make_svc);

    // Finally, spawn `server` onto an Executor...
    if let Err(e) = hyper::rt::spawn(server) {
        eprintln!("an error occurred: {}", e);
    }
}

async fn handle_request(req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    Ok(match Multipart::try_from_request(req) {
        Ok(multipart) => match handle_multipart(multipart).await {
            Ok(()) => Response::new(Body::from("successful request!")),
            Err(e) => Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from(e.to_string()))?
        }
        Err(req) => Response::new(Body::from("expecting multipart/form-data"))
    })
}

async fn handle_multipart(mut multipart: Multipart<Body>) -> Result<(), hyper::Error> {
    while let Some(mut field) = multipart.next_field().await? {
        println!("got field: {:?}", field.headers);

        while let Some(chunk) = field.data.try_next().await? {
            println!("got field chunk: {:?}", String::from_utf8_lossy(chunk))      ;
        }
    }

    Ok(())
}
