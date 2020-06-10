use httparse::Error::Status;
use hyper::rt;
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use multipart_async::server::Multipart;
use std::net::TcpStream;

use futures::{Future, FutureExt, TryStreamExt};

type Error = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main]
async fn main() {
    let addr = ([127, 0, 0, 1], 3000).into();

    let make_svc = make_service_fn(|socket: &AddrStream| {
        println!("\n\nrequest from: {}", socket.remote_addr());

        async move { Ok::<_, Error>(service_fn(handle_request)) }
    });

    // Then bind and serve...
    let server = Server::bind(&addr).serve(make_svc);

    println!("server running on {}", server.local_addr());

    if let Err(e) = server.await {
        eprintln!("an error occurred: {}", e);
    }
}

async fn handle_request(req: Request<Body>) -> Result<Response<Body>, Error> {
    Ok(match Multipart::try_from_request(req) {
        Ok(multipart) => match handle_multipart(multipart).await {
            Ok(()) => Response::new(Body::from("successful request!")),
            Err(e) => Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from(e.to_string()))?,
        },
        Err(req) => Response::new(Body::from("expecting multipart/form-data")),
    })
}

async fn handle_multipart(mut multipart: Multipart<Body>) -> Result<(), Error> {
    while let Some(mut field) = multipart.next_field().await? {
        println!("got field: {:?}", field.headers);

        if field.headers.is_text() {
            println!("field text: {:?}", field.data.read_to_string().await?);
        } else {
            while let Some(chunk) = field.data.try_next().await? {
                println!("got field chunk, len: {:?}", chunk.len());
            }
        }
    }

    Ok(())
}
