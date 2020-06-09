// Copyright 2017-2019 `multipart-async` Crate Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
use std::error::Error;
use std::future::Future;
use std::io::{Cursor};
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use futures_util::TryStreamExt;
use http::header::HeaderName;
use mime::Mime;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub struct MultipartWriter<W> {
    inner: W,
    boundary: String,
    data_written: bool,
}

impl<W> MultipartWriter<W> {
    pub(crate) fn new(inner: W, boundary: String) -> Self {
        MultipartWriter {
            inner,
            boundary,
            data_written: false,
        }
    }

    fn get_field_header(
        &self,
        name: &str,
        filename: Option<&str>,
        content_type: Option<&Mime>,
    ) -> String {
        use std::fmt::Write;

        let mut header = format!(
            "--{}\r\nContent-Disposition: form-data; name=\"{}\"",
            self.boundary, name
        );

        if let Some(filename) = filename {
            write!(header, "; filename=\"{}\"", filename).unwrap();
        }

        if let Some(content_type) = content_type {
            write!(header, "\r\nContent-Type: {}", content_type);
        }

        header.push_str("\r\n\r\n");

        header
    }

    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: AsyncWrite + Unpin> MultipartWriter<W> {
    async fn write_field_header(
        &mut self,
        name: &str,
        filename: Option<&str>,
        content_type: Option<&Mime>,
    ) -> io::Result<()> {
        let mut header = Cursor::new(self.get_field_header(name, filename, content_type));
        io::copy(&mut header, &mut self.inner).await?;
        self.data_written = true;
        Ok(())
    }

    /// Write a field of any type to the output. (Method for taking `AsyncRead`).
    ///
    /// If `content_type` is not set, the server assumes `Content-Type: text/plain`
    /// ([RFC 7578 Section 4.4][7578-4.4]).
    ///
    /// Typically, if `filename` and `content_type` are omitted, the server will interpret it as a
    /// non-file text field like with `application/x-www-form-urlencoded` form fields. Unless
    /// a charset is manually specified, the server *should* assume the text encoding is UTF-8.
    ///
    /// If `filename` is provided even if `content_type` is not set, it may cause the
    /// server to interpret the field as a text file instead of a text field.
    ///
    /// If you want the server to interpret a field as a file regardless of type or filename,
    /// pass a `content_type` of `mime::APPLICATION_OCTET_STREAM`.
    ///
    /// [7578-4.4]: https://tools.ietf.org/html/rfc7578#section-4.4
    pub async fn write_field<R: AsyncRead + Unpin>(
        &mut self,
        name: &str,
        filename: Option<&str>,
        content_type: Option<&Mime>,
        mut contents: R,
    ) -> io::Result<&mut Self> {
        self.write_field_header(name, filename, content_type)
            .await?;
        io::copy(&mut contents, &mut self.inner).await?;
        self.inner.write_all(b"\r\n").await?;
        Ok(self)
    }

    /// Like [`.write_field()`](#method.write_field) but takes a `Stream`.
    /// See that method for details on these parameters.
    ///
    /// Errors from the stream will be wrapped as `io::ErrorKind::Other`.
    pub async fn write_stream<B, E, S>(
        &mut self,
        name: &str,
        filename: Option<&str>,
        content_type: Option<&Mime>,
        mut contents: S,
    ) -> io::Result<&mut Self>
    where
        B: AsRef<[u8]>,
        E: Into<Box<dyn Error + Send + Sync>>,
        S: Stream<Item = Result<B, E>> + Unpin,
    {
        let mut contents = contents.map_err(|e| io::Error::new(io::ErrorKind::Other, e));

        while let Some(buf) = contents.try_next().await? {
            self.inner.write_all(buf.as_ref()).await?;
        }

        self.inner.write_all(b"\r\n").await?;
        Ok(self)
    }

    /// Open a file for reading and copy it as a field to the output, inferring the filename
    /// and content-type from the path.
    ///
    /// If no content-type is known for the path extension or there is no extension,
    /// `application/octet-stream` is assumed to ensure the server interprets this field as a file.
    ///
    /// If you want to override the filename or content-type, use
    /// [`.write_field()`](#method.write_field) instead.
    #[cfg(feature = "tokio-fs")]
    pub async fn write_file<P: AsRef<Path>>(
        &mut self,
        name: &str,
        path: P,
    ) -> io::Result<&mut Self> {
        let path = path.as_ref();
        let filename = path.file_name().and_then(|s| s.to_str());
        let content_type = mime_guess::from_path(path).first_or_octet_stream();
        let file = tokio_fs::File::open(path)?;

        self.write_field(name, filename, Some(&content_type), file)
    }

    /// Write a plain text field to the output.
    ///
    /// The server must assume `Content-Type: text/plain` ([RFC 7578 Section 4.4][7578-4.4]).
    /// Typically, the server will interpret it as a non-file text field like with
    /// `application/x-www-form-urlencoded` form fields.
    ///
    /// If you want to pass a string but still set the filename and/or content type,
    /// convert it to bytes with `.as_bytes()` and pass it to [`.write_field()`](#method.write_field)
    /// instead, as byte slices implement `AsyncRead`.
    pub async fn write_text(&mut self, name: &str, text: &str) -> io::Result<&mut Self> {
        self.write_field(name, None, None, text.as_bytes()).await
    }

    /// Complete the `multipart/form-data` request.
    ///
    /// Writes the trailing boundary and flushes the output.
    ///
    /// The request should be closed at this point as the server must ignore all data outside
    /// the multipart body.
    pub async fn finish(&mut self) -> io::Result<()> {
        if self.data_written {
            self.inner.write_all(b"--").await?;
            self.inner.write_all(self.boundary.as_bytes()).await?;
            // trailing newline isn't necessary per the spec but some clients are expecting it
            // https://github.com/actix/actix-web/issues/598
            self.inner.write_all(b"--\r\n").await?;
        }

        self.inner.flush().await?;

        Ok(())
    }
}

#[cfg(test)]
#[tokio::test]
async fn test_multipart_writer_one_text_field() -> io::Result<()> {
    let mut writer = MultipartWriter {
        inner: Vec::<u8>::new(),
        boundary: "boundary".to_string(),
        data_written: false,
    };

    writer.write_text("hello", "world!").await?.finish().await?;

    assert_eq!(
        writer.inner,
        &b"--boundary\r\n\
          Content-Disposition: form-data; name=\"hello\"\r\n\r\n\
          world!\r\n\
          --boundary--\r\n"[..]
    );

    Ok(())
}
