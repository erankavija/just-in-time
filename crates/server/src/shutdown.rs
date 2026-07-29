//! Graceful shutdown of the serving process.
//!
//! Two mechanisms cooperate, in this order:
//!
//! 1. One [`CancellationToken`], cloned into the application state and from
//!    there into every live event stream. Cancelling it ends those streams, so
//!    an idle SSE subscriber reaches EOF immediately instead of holding its
//!    connection open behind a 15-second keepalive.
//! 2. One [`axum_server::Handle`], which owns the listener and the connections.
//!    [`Handle::graceful_shutdown`] with a deadline stops acceptance, gives
//!    everything still running at most [`GRACEFUL_DRAIN_TIMEOUT`] to finish, and
//!    force-closes whatever is left when the deadline expires — the bound that
//!    makes process exit predictable even for a connection that never completes.
//!
//! [`run_shutdown_sequence`] performs both, in that order, and reports what the
//! drain deadline saw.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use tokio::time::{sleep, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Longest an ordinary connection may keep running after a shutdown signal.
///
/// Survivors are force-closed once it expires, so the process exits on a
/// bounded schedule rather than waiting on a peer that never finishes.
pub const GRACEFUL_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the drain is sampled while the deadline runs down.
const DRAIN_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// The server handle this process shuts down through.
pub type ServerHandle = axum_server::Handle<SocketAddr>;

/// The registered signal that asked the process to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownSignal {
    /// Ctrl-C at the terminal (`SIGINT` on Unix).
    Interrupt,
    /// A termination request from a supervisor (`SIGTERM`).
    Terminate,
}

impl ShutdownSignal {
    /// The platform name the signal is logged under.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
        }
    }
}

impl std::fmt::Display for ShutdownSignal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What the drain deadline saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainOutcome {
    /// Every connection finished on its own before the deadline.
    Drained,
    /// Connections were still open when the deadline expired, and were closed
    /// by the server rather than by their peers.
    ForcedClosed {
        /// How many were still open as the deadline ran out.
        connections: usize,
    },
}

/// Resolves when this process is asked to stop, reporting which signal arrived.
///
/// On Unix both Ctrl-C and `SIGTERM` are registered; elsewhere only Ctrl-C is
/// available.
///
/// # Errors
/// Returns the operating system's error if a handler cannot be registered.
/// Registration happens on the first poll, so the error surfaces here rather
/// than silently leaving the process unsignallable.
pub async fn await_shutdown_signal() -> io::Result<ShutdownSignal> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.map(|()| ShutdownSignal::Interrupt),
            _ = terminate.recv() => Ok(ShutdownSignal::Terminate),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .map(|()| ShutdownSignal::Interrupt)
    }
}

/// Waits for `signal`, then shuts the server down within `drain_deadline`.
///
/// Cancels `shutdown` first so live event streams end and stop counting against
/// the drain, then hands the deadline to `handle`, which stops accepting
/// connections, lets the rest finish, and force-closes whatever survives to the
/// deadline. The serve future returns `Ok` on both paths.
///
/// # Errors
/// Returns `signal`'s error when the process could not register its signal
/// handlers. A process that cannot be asked to stop is not left serving: the
/// same shutdown runs, and the error reaches the caller so it becomes the
/// process's exit status.
pub async fn run_shutdown_sequence<F>(
    handle: ServerHandle,
    shutdown: CancellationToken,
    drain_deadline: Duration,
    signal: F,
) -> io::Result<DrainOutcome>
where
    F: Future<Output = io::Result<ShutdownSignal>>,
{
    let signal = match signal.await {
        Ok(signal) => signal,
        Err(error) => {
            warn!(%error, "Shutdown signal handling failed; stopping the server");
            shutdown.cancel();
            handle.graceful_shutdown(Some(drain_deadline));
            return Err(error);
        }
    };

    info!(
        %signal,
        drain_deadline_secs = drain_deadline.as_secs(),
        open_connections = handle.connection_count(),
        "Shutdown signal received; draining connections"
    );

    // Order matters: event streams end first, so the drain below is only ever
    // waiting on connections that have real work left.
    shutdown.cancel();
    handle.graceful_shutdown(Some(drain_deadline));

    let outcome = observe_drain(&handle, drain_deadline).await;
    match outcome {
        DrainOutcome::Drained => info!("All connections finished before the drain deadline"),
        DrainOutcome::ForcedClosed { connections } => warn!(
            open_connections = connections,
            drain_deadline_secs = drain_deadline.as_secs(),
            "Drain deadline expired; force-closing the connections that did not finish"
        ),
    }
    Ok(outcome)
}

/// Watches the live connection count until `deadline` expires.
///
/// The count is sampled up to one interval before expiry and never after it:
/// once the deadline forces the survivors closed the count reads zero, which is
/// indistinguishable from a voluntary finish.
async fn observe_drain(handle: &ServerHandle, deadline: Duration) -> DrainOutcome {
    let expiry = Instant::now() + deadline;
    loop {
        let remaining = expiry.saturating_duration_since(Instant::now());
        if remaining <= DRAIN_SAMPLE_INTERVAL {
            let connections = handle.connection_count();
            sleep(remaining).await;
            return match connections {
                0 => DrainOutcome::Drained,
                connections => DrainOutcome::ForcedClosed { connections },
            };
        }
        sleep(DRAIN_SAMPLE_INTERVAL).await;
        if handle.connection_count() == 0 {
            return DrainOutcome::Drained;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use std::io;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;
    use tokio_util::sync::CancellationToken;

    /// Bound on every wait a failing test would otherwise spend hanging.
    const TEST_BUDGET: Duration = Duration::from_secs(5);

    /// Serves a minimal app on an ephemeral loopback port through `handle`.
    async fn serve_on_loopback(handle: &ServerHandle) -> (SocketAddr, JoinHandle<io::Result<()>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let app = Router::new().route("/health", get(|| async { "ok" }));
        let server = axum_server::from_tcp(listener)
            .expect("adopt the bound listener")
            .handle(handle.clone());
        let serving = tokio::spawn(server.serve(app.into_make_service()));
        let addr = timeout(TEST_BUDGET, handle.listening())
            .await
            .expect("the server binds within the test budget")
            .expect("the server reports its bound address");
        (addr, serving)
    }

    /// Opens a connection whose request never completes: the request line and a
    /// header are sent, the terminating blank line never is. Hyper stays mid
    /// message, so a graceful shutdown cannot retire the connection and only
    /// the drain deadline can close it.
    async fn open_stalled_connection(addr: SocketAddr) -> TcpStream {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n")
            .await
            .expect("send a partial request");
        stream.flush().await.expect("flush the partial request");
        stream
    }

    /// Opens a connection, completes one request/response exchange on it, and
    /// leaves it idle in HTTP keep-alive.
    async fn open_completed_connection(addr: SocketAddr) -> TcpStream {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("send a complete request");
        let mut response = Vec::new();
        while !response.ends_with(b"ok") {
            let mut chunk = [0u8; 256];
            let read = timeout(TEST_BUDGET, stream.read(&mut chunk))
                .await
                .expect("the response arrives within the test budget")
                .expect("read the response");
            assert_ne!(read, 0, "the server closed before answering the request");
            response.extend_from_slice(&chunk[..read]);
        }
        stream
    }

    /// Waits until the handle reports `expected` live connections.
    async fn wait_for_connection_count(handle: &ServerHandle, expected: usize) {
        let observed = timeout(TEST_BUDGET, async {
            loop {
                let count = handle.connection_count();
                if count == expected {
                    return count;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        assert_eq!(
            observed.ok(),
            Some(expected),
            "the server never reported {expected} live connection(s)"
        );
    }

    #[tokio::test]
    async fn test_run_shutdown_sequence_force_closes_connections_that_outlive_the_deadline() {
        let handle = ServerHandle::new();
        let (addr, serving) = serve_on_loopback(&handle).await;
        let mut stalled = open_stalled_connection(addr).await;
        wait_for_connection_count(&handle, 1).await;
        let shutdown = CancellationToken::new();

        let outcome = run_shutdown_sequence(
            handle.clone(),
            shutdown.clone(),
            Duration::from_millis(300),
            std::future::ready(Ok(ShutdownSignal::Terminate)),
        )
        .await
        .expect("a delivered signal is not an error");

        assert_eq!(outcome, DrainOutcome::ForcedClosed { connections: 1 });
        assert!(
            shutdown.is_cancelled(),
            "event streams must be cancelled so they do not hold the drain open"
        );
        timeout(TEST_BUDGET, serving)
            .await
            .expect("the serve future ends within the test budget")
            .expect("the serve task does not panic")
            .expect("the serve future returns Ok after a forced close");
        let mut byte = [0u8; 1];
        let read = timeout(TEST_BUDGET, stalled.read(&mut byte))
            .await
            .expect("the stalled connection is closed within the test budget")
            .expect("read the closed connection");
        assert_eq!(read, 0, "the stalled connection must be force-closed");
    }

    #[tokio::test]
    async fn test_run_shutdown_sequence_reports_a_voluntary_drain_before_the_deadline() {
        let handle = ServerHandle::new();
        let (addr, serving) = serve_on_loopback(&handle).await;
        let _completed = open_completed_connection(addr).await;
        wait_for_connection_count(&handle, 1).await;

        let outcome = timeout(
            TEST_BUDGET,
            run_shutdown_sequence(
                handle.clone(),
                CancellationToken::new(),
                GRACEFUL_DRAIN_TIMEOUT,
                std::future::ready(Ok(ShutdownSignal::Interrupt)),
            ),
        )
        .await
        .expect("an idle keep-alive connection drains well inside the deadline")
        .expect("a delivered signal is not an error");

        assert_eq!(outcome, DrainOutcome::Drained);
        timeout(TEST_BUDGET, serving)
            .await
            .expect("the serve future ends within the test budget")
            .expect("the serve task does not panic")
            .expect("the serve future returns Ok after a voluntary drain");
    }

    #[tokio::test]
    async fn test_run_shutdown_sequence_propagates_a_signal_registration_error() {
        let handle = ServerHandle::new();
        let (_addr, serving) = serve_on_loopback(&handle).await;
        let shutdown = CancellationToken::new();

        let error = run_shutdown_sequence(
            handle.clone(),
            shutdown.clone(),
            Duration::from_millis(100),
            std::future::ready(Err(io::Error::other("signal registration failed"))),
        )
        .await
        .expect_err("a failed signal registration must not be reported as a clean shutdown");

        assert!(
            error.to_string().contains("signal registration failed"),
            "the registration error must reach the caller verbatim, got: {error}"
        );
        assert!(
            shutdown.is_cancelled(),
            "a process that cannot be signalled must still stop serving"
        );
        timeout(TEST_BUDGET, serving)
            .await
            .expect("the server stops rather than serving unstoppably")
            .expect("the serve task does not panic")
            .expect("the serve future returns Ok");
    }

    #[tokio::test]
    async fn test_await_shutdown_signal_registers_handlers_without_error() {
        // Registration failures surface on the first poll, so an elapsed
        // timeout is the observable proof that the handlers are installed and
        // waiting. No signal is delivered to the test process.
        let outcome = timeout(Duration::from_millis(100), await_shutdown_signal()).await;

        assert!(
            outcome.is_err(),
            "signal registration failed instead of waiting: {:?}",
            outcome.map(|result| result.map_err(|error| error.to_string()))
        );
    }
}
