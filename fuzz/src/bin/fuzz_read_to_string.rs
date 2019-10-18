#[macro_use] extern crate afl;
extern crate multipart_async;

fn main() {
    fuzz!(|data: &[u8]| {
        multipart_async::fuzzing::fuzz_read_to_string(data)
    })
}
