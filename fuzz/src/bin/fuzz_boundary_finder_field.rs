//! Fuzz BoundaryFinder using the input data as field contents
#[macro_use] extern crate afl;
extern crate multipart_async;

fn main() {
    fuzz!(|data: &[u8]| {
        multipart_async::fuzzing::fuzz_boundary_finder_field(data)
    })
}
