extern crate hyper;
extern crate multipart_async;

use hyper::{Control, Encoder, Decoder, Next, Server, StatusCode};
use hyper::net::Transport;
use hyper::server::{Handler, Request, Response};

use multipart_async::server::{self, Field, FieldData, Multipart};
use multipart_async::server::Error::*;

fn main() {
    let addr = "127.0.0.1:8080".parse().unwrap();

    let (listen, sloop) = Server::http(&addr).unwrap()
        .handle(|ctrl: Control| {
            ctrl.ready(Next::read()).unwrap();
            MultipartHandler::default()
        })
        .unwrap();

    sloop.run();
}

#[derive(Default)]
struct MultipartHandler {
    multipart: Option<Multipart>,
}

impl<T: Transport> Handler<T> for MultipartHandler {
    fn on_request(&mut self, req: Request<T>) -> Next {
        self.multipart = Some(Multipart::from_request(&req)
            .expect("Expected a multipart request"));

        Next::read()
    }

    fn on_request_readable(&mut self, req: &mut Decoder<T>) -> Next {
        let mut multipart = self.multipart.as_mut().unwrap();

        multipart.read_from(req).unwrap();

        loop {
            if let Some((field, data)) = multipart.get_field() {
                match write_field(field, data) {
                    Ok(_) => (),
                    Err(AtEnd) => return Next::end(),
                    Err(ReadMore) => break,
                }
            }

            match multipart.next_field()
                .and_then(|(field, data)| write_field(field, data)) {
                Ok(_) => continue,
                Err(AtEnd) => return Next::end(),
                Err(ReadMore) => break,
            }
        }

        Next::read()
    }

    fn on_response(&mut self, res: &mut Response) -> Next {
        res.set_status(StatusCode::Ok);
        Next::read()
    }

    fn on_response_writable(&mut self, _: &mut Encoder<T>) -> Next {
        Next::read()
    }
}

fn write_field(f: &Field, mut d: FieldData) -> server::Result<()> {
    if f.is_text() {
        println!("{} : {}", f.name(), try!(d.read_string()).unwrap());
    } else {
        println!("{} : (binary)", f.name());
    }

    Ok(())
}
