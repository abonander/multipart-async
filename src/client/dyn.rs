/// Dynamically constructed multipart bodies.

use buf_redux::BufReader;

use mime::Mime;

use std::borrow::Cow;
use std::fs::File;
use std::io::prelude::*;
use std::io;
use std::path::Path;


use super::*;

use super::RequestStatus::*;

#[derive(Default)]
pub struct Dynamic<'a> {
    fields: Vec<DynField<'a>>    
}

impl<'a> Dynamic<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text<T: Into<Cow<'a, str>> + 'a>(&mut self, name: &str, val: T) -> &mut Self {
        let field = TextField::new(name, val);

        self.fields.push(DynField::Text(field));
        self
    }

    pub fn open_file<P: AsRef<Path>>(&mut self, name: &str, path: P) -> &mut Self {
        self.try_open_file(name, &path).unwrap_or_else(|err| 
            panic!("Failed to open file at {}. Error: {:?}", path.as_ref().display(), err)
        )
    }

    pub fn try_open_file<P: AsRef<Path>>(&mut self, name: &str, path: P) -> io::Result<&mut Self> {
        let field = try!(FileField::open_file(name, path));

        self.fields.push(DynField::File(field));

        Ok(self)
    }

    pub fn stream<R: Read + 'a>(&mut self, name: &str, stream: R, content_type: Option<&Mime>, filename: Option<&str>) -> &mut Self {
        let field = StreamField::new(name, stream, content_type, filename);
        self.fields.push(DynField::Stream(field.boxed()));
        self
    }

    pub fn stream_buf<B: BufRead + 'a>(&mut self, name: &str, stream: B, content_type: Option<&Mime>, filename: Option<&str>) -> &mut Self {
        let field = StreamField::buffered(name, stream, content_type, filename);
        self.fields.push(DynField::Buffered(field.boxed_buffered()));
        self
    }
}

impl<'a> Body for Dynamic<'a> {
    fn finished(&self) -> bool {
        self.fields.is_empty()
    }

    fn pop_field(&mut self) {
        self.fields.pop();
    }

    fn write_field<W: Write>(&mut self, out: &mut W) -> io::Result<RequestStatus> {
        self.fields.last_mut()
            .map_or(Ok(NullRead), |field| field.write_out(out)) 
    }
}

enum DynField<'a> {
    Text(TextField<'a>),
    File(FileField),
    Buffered(StreamField<Box<BufRead + 'a>>),
    Stream(StreamField<BufReader<Box<Read + 'a>>>),
}

impl<'a> Field for DynField<'a> {
    fn write_out<W: Write>(&mut self, out: &mut W) -> io::Result<FieldStatus> {
        use self::DynField::*;
    
        match *self {
            Text(ref mut text) => text.write_out(out),
            File(ref mut file) => file.write_out(out),
            Buffered(ref mut buf) => buf.write_out(out),
            Stream(ref mut stream) => stream.write_out(out),
        }
    }
}
