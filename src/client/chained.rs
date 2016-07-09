use std::io::prelude::*;

use std::borrow::Cow;

use std::fs::File;

use super::MultipartWriter as Writer;

use super::RequestStatus as FieldStatus;

pub trait AbsFieldLink {
    fn write_next(&mut self, wrt: &mut Writer) -> io::Result<FieldStatus>;

    fn link<F: Field>(self, new_field: F) -> FieldLink<F, Self> {
        FieldLink {
            field: Some(new_field),
            next: self,
        }
    }
}

impl AbsFieldLink for () {
    fn write_next(&mut self, _: &mut Writer) -> io::Result<FieldStatus> {
        Ok(FieldStatus::Done)
    }
}

pub struct FieldLink<F, N> {
    field: Option<F>,
    next: N,
}

impl<F: Field, N: AbsFieldLink> AbsFieldLink for FieldLink<F, N> {
    fn write_next(&mut self, wrt: &mut Writer) -> io::Result<FieldStatus> {
        if let Some(ref mut field) = self.field {
            if let FieldStatus::MoreData = try!(field.write_out(wrt)) {
                return Ok(FieldStatus::MoreData);
            }
        }

        self.field = None;

        self.next.write_next(wrt)
    }
}



