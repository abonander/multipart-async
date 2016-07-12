use std::io::prelude::*;

use std::borrow::Cow;

use std::fs::File;
use std::io;

use super::{Body, Field, RequestStatus, FieldStatus};

#[macro_export]
macro_rules! chained {
    ($($name:expr => $typ:ident($val:tt)),+,) => (
        {
            use $crate::client::chained::{Chain, AbsFieldLink};
        
            Chain$(.link(field!($name => $typ($val))))+.into()
        }
    );
    ($($name:expr => $typ:ident($val:tt)),+) => (
        {
            use $crate::client::chained::{Chain, AbsFieldLink};
        
            Chain$(.link(field!($name => $typ($val))))+.into()
        }
    )
}

pub trait AbsFieldLink: Body + Sized {
    fn link<F: Field>(self, new_field: F) -> FieldLink<F, Self> {
        FieldLink {
            field: Some(new_field),
            next: self,
        }
    }
}

pub struct Chain;

impl Body for Chain {
    fn write_field<W: Write>(&mut self, _: &mut W) -> io::Result<FieldStatus> {
        // FIXME: can't use enum values through type alias on 1.12 nightly
        Ok(RequestStatus::NullRead)
    }

    fn pop_field(&mut self) {}

    fn finished(&self) -> bool { true }
}

impl AbsFieldLink for Chain {}

pub struct FieldLink<F, N> {
    field: Option<F>,
    next: N,
}

impl<F: Field, N: AbsFieldLink> Body for FieldLink<F, N> {
    #[inline(always)]
    fn write_field<W: Write>(&mut self, wrt: &mut W) -> io::Result<FieldStatus> {
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

impl<F: Field, N: AbsFieldLink> AbsFieldLink for FieldLink<F, N> {}

