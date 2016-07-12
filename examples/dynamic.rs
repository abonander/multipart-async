extern crate multipart_async;

use multipart_async::client::*;
use multipart_async::client::dyn::Dynamic;

use std::io;

fn main() {
    let stream = (0..32).collect::<Vec<u8>>();

    let mut dynamic = Dynamic::new();

    let mut multipart = dynamic
        .text("text", "Hello, world!")
        .open_file("file", "lorem_ipsum.txt")
        .stream_buf("binary", &*stream, None, None)
        .build();

    let mut stdout = io::stdout();

    while let RequestStatus::MoreData = multipart.on_writable(&mut stdout).unwrap() {}
}
