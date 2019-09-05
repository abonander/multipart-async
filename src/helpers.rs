use std::{fmt, mem};

pub fn replace_default<T: Default>(dest: &mut T) -> T {
    mem::replace(dest, T::default())
}

pub fn show_bytes(bytes: &[u8]) -> impl fmt::Display + '_ {
    display_bytes::HEX_UTF8.clone()
        .escape_control(true)
        .min_str_len(1).display_bytes(bytes)
}
