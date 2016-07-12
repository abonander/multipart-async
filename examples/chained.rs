#[macro_use] extern crate multipart_async;

use multipart_async::client::*;

use std::io;

fn main() {
    let mut stdout = io::stdout();

    let stream: Vec<_> = (0u8 .. 32).collect();

    let mut multipart = chained! {
        "text" => text("Hello, world!"),
        "file" => file("lorem_ipsum.txt"),
        "binary" => stream((io::Cursor::new(stream))),
    };

    while let RequestStatus::MoreData = multipart.on_writable(&mut stdout).unwrap() {}
}

