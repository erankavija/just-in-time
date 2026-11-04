//! Test-support seam that reports each connection's first received bytes.
//!
//! A connection the accept loop has registered can still sit in hyper-util's
//! protocol detection, and a graceful shutdown cancels it there rather than
//! leaving it for the drain deadline. The graceful-shutdown suite needs a
//! connection past that point, and only the server can say it got there: the
//! report is written from the read that ends detection, inside the same poll
//! that hands the connection to the HTTP/1 state machine.

use std::future::{ready, Ready};
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum_server::accept::Accept;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// The variable that switches reporting on for a spawned server.
pub const FIRST_READ_REPORT_ENV: &str = "JIT_SERVER_REPORT_FIRST_READS";

/// The stderr line prefix of a report; the peer address follows after a space.
pub const FIRST_READ_EVENT: &str = "jit-server-first-read";

/// Wraps accepted streams so the first read that returns bytes is reported
/// on stderr as `FIRST_READ_EVENT <peer>`, when [`FIRST_READ_REPORT_ENV`] is set.
#[derive(Debug, Clone, Copy)]
pub struct FirstReadAcceptor {
    reporting: bool,
}

impl FirstReadAcceptor {
    /// Reports only when [`FIRST_READ_REPORT_ENV`] is set in this process.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            reporting: std::env::var_os(FIRST_READ_REPORT_ENV).is_some(),
        }
    }
}

impl<S> Accept<TcpStream, S> for FirstReadAcceptor {
    type Stream = FirstReadStream;
    type Service = S;
    type Future = Ready<io::Result<(FirstReadStream, S)>>;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let unreported = if self.reporting {
            stream.peer_addr().map(Some)
        } else {
            Ok(None)
        };
        ready(unreported.map(|unreported| (FirstReadStream { stream, unreported }, service)))
    }
}

/// An accepted stream that reports its peer once, on the first read with bytes.
#[derive(Debug)]
pub struct FirstReadStream {
    stream: TcpStream,
    unreported: Option<SocketAddr>,
}

impl AsyncRead for FirstReadStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.stream).poll_read(cx, buf);
        if buf.filled().len() > before {
            if let Some(peer) = self.unreported.take() {
                eprintln!("{FIRST_READ_EVENT} {peer}");
            }
        }
        polled
    }
}

impl AsyncWrite for FirstReadStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
