use std::io::prelude::*;

use std::borrow::Cow;

use std::fs::File;

use super::Body;

use super::FieldStatus;

#[macro_export]
macro_rules! chain {
    ($($name:expr => $type:ident($val:tt)),+) => (
        {
            use $crate::multipart_async::client::chained::{Chain, AbsFieldLink};
        
            Chain$(.link(field!($name => $type($val))))+
        }
    )
}

macro_rules! field (
    ($name:expr => text($val:expr)) => (
        $crate::multipart_async::client::TextField::new($name, $val)
    );
    ($name:expr => file($val:expr)) => (
        
);

pub trait AbsFieldLink: Body {
    fn link<F: Field>(self, new_field: F) -> FieldLink<F, Self> {
        FieldLink {
            field: Some(new_field),
            next: self,
        }
    }
}

pub struct Chain;

impl AbsFieldLink for Chain {
    fn write_field(&mut self, _: &mut Writer) -> io::Result<FieldStatus> {
        Ok(FieldStatus::Done)
    }

    fn pop_field(&self) {}

    fn finished(&self) -> bool { true }
}

pub struct FieldLink<F, N> {
    field: Option<F>,
    next: N,
}

impl<F: Field> FieldLink<F, ()> {
    pub fn start(field: F) -> Self {
        FieldLink {
            field: Some(field),
            next: (),
        }
    }
}

impl<F: Field, N: AbsFieldLink> AbsFieldLink for FieldLink<F, N> {
    #[inline(always)]
    fn write_field(&mut self, wrt: &mut Writer) -> io::Result<FieldStatus> {
        if let Some(ref mut field) = self.field {
            field.write_out(wrt)
        } else {
            self.next.write_field(wrt)
        }
    }

    #[inline(always)]
    fn pop_field(&mut self) {
        if self.field.is_some() {
            self.field = None;
        } else {
            self.next.pop_field();
        }
    }

    #[inline(always)]
    fn finished(&self) -> bool {
        self.field.is_none() && self.next.finished()
    }
}



