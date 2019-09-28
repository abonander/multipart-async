use std::io::{self, Cursor};
use std::pin::Pin;

use http::header::HeaderName;
use http::HeaderValue;
use mime::Mime;
use rand::distributions::{Alphanumeric, Distribution};

use tokio_io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::task::{Poll, Context};
use std::future::Future;

const BOUNDARY_LEN: usize = 32;

pub struct MultipartWriter<W> {
    inner: W,
    boundary: String,
    data_written: bool,
}

impl<W> MultipartWriter<W> {
    pub fn new(inner: W) -> Self {
        let mut boundary = String::with_capacity(BOUNDARY_LEN);
        boundary.extend(Alphanumeric.sample_iter(rand::thread_rng()).take(BOUNDARY_LEN));

        MultipartWriter {
            inner,
            boundary,
            data_written: false,
        }
    }

    pub fn get_content_type(&self) -> HeaderValue {
        format!("multipart/form-data; boundary={}", self.boundary)
            .parse().expect("this should be a valid header value")
    }

    fn get_field_header(&self, name: &str, filename: Option<&str>, content_type: Option<&Mime>) -> String {
        use std::fmt::Write;

        let mut header = format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"", self.boundary, name);

        if let Some(filename) = filename {
            write!(header, "; filename=\"{}\"", filename).unwrap();
        }

        if let Some(content_type) = content_type {
            write!(header, "\r\nContent-Type: {}", content_type);
        }

        header.push_str("\r\n\r\n");

        header
    }
}

impl<W: AsyncWrite + Unpin> MultipartWriter<W> {
    async fn write_field<R: AsyncRead + Unpin>(&mut self, name: &str, filename: Option<&str>, content_type: Option<&Mime>, mut contents: R) -> io::Result<&mut Self> {
        let mut header = Cursor::new(self.get_field_header(name, filename, content_type));
        header.copy(&mut self.inner).await?;
        self.data_written = true;
        contents.copy(&mut self.inner).await?;
        self.inner.write_all(b"\r\n").await?;
        Ok(self)
    }

    pub async fn write_text(&mut self, name: &str, text: &str) -> io::Result<&mut Self> {
        self.write_field(name, None, None, text.as_bytes()).await
    }

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

#[test]
fn test_multipart_writer_get_content_type() {
    let writer = MultipartWriter {
        inner: (),
        boundary: "boundary".to_string(),
        data_written: false,
    };

    assert_eq!(writer.get_content_type(), "multipart/form-data; boundary=boundary");
}

#[cfg(test)]
#[tokio::test]
async fn test_multipart_writer_one_text_field() -> io::Result<()> {
    let mut writer = MultipartWriter {
        inner: Vec::<u8>::new(),
        boundary: "boundary".to_string(),
        data_written: false,
    };

    writer.write_text("hello", "world!").await?
        .finish().await?;

    assert_eq!(
        writer.inner,
        &b"--boundary\r\n\
          Content-Disposition: form-data; name=\"hello\"\r\n\r\n\
          world!\r\n\
          --boundary--\r\n"[..]
    );

    Ok(())
}
