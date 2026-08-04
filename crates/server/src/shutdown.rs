//! Graceful shutdown of the serving process.
//!
//! Two mechanisms cooperate, in this order:
//!
//! 1. One [`CancellationToken`], cloned into the application state and from
//!    there into every live event stream. Cancelling it ends those streams, so
//!    an idle SSE subscriber reaches EOF immediately instead of holding its
//!    connection open behind a 15-second keepalive.
//! 2. One [`axum_server::Handle`], which owns the listener and the connections.
//!    [`axum_server::Handle::graceful_shutdown`] stops acceptance without
//!    starting a second timer. JIT owns the sole [`GRACEFUL_DRAIN_TIMEOUT`]
//!    boundary, samples the live count there, and calls
//!    [`axum_server::Handle::shutdown`] when survivors remain. That makes
//!    process exit predictable even when axum-server's serving task observes
//!    the graceful notification late.
//!
//! [`run_shutdown_sequence`] performs both, in that order, and reports what the
//! drain deadline saw and how long the drain ran to see it.

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

/// Renders the platform name the signal is logged under.
impl std::fmt::Display for ShutdownSignal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
        })
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
/// the drain, then asks `handle` to stop accepting and drain indefinitely. JIT
/// owns the only deadline: at that boundary it samples the remaining count and
/// invokes the handle's immediate shutdown if any connections survive. The
/// serve future returns `Ok` on both paths.
///
/// Either outcome is logged with `drained_for_ms`, the time the drain actually
/// spent running. It is measured across the drain rather than restated from
/// `drain_deadline`, so the log distinguishes a server that waited out its
/// deadline from one that stopped as soon as it had nothing left to drain.
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
            handle.shutdown();
            return Err(error);
        }
    };

    info!(
        %signal,
        drain_deadline_secs = drain_deadline.as_secs(),
        open_connections = handle.connection_count(),
        "Shutdown signal received; draining connections"
    );

    // Stamp the start before the boundary exists, on the clock the boundary is
    // scheduled against: `sleep` measures from its own creation and never
    // completes early, so an elapsed time taken from here can only overstate the
    // wait a connection received, never understate it.
    let drain_started = Instant::now();
    // Create JIT's sole boundary before notification. axum-server drains
    // indefinitely; it cannot start a second deadline when its serving task
    // eventually observes the graceful notification.
    let boundary = sleep(drain_deadline);
    let outcome = drain_connections(&handle, &shutdown, boundary).await;
    let drained_for = drain_started.elapsed();
    match outcome {
        DrainOutcome::Drained => info!(
            drained_for_ms = drained_for.as_millis(),
            "All connections finished before the drain deadline"
        ),
        DrainOutcome::ForcedClosed { connections } => warn!(
            open_connections = connections,
            drain_deadline_secs = drain_deadline.as_secs(),
            drained_for_ms = drained_for.as_millis(),
            "Drain deadline expired; force-closing the connections that did not finish"
        ),
    }
    Ok(outcome)
}

/// Cancels application streams, initiates an unbounded graceful drain in
/// axum-server, and enforces JIT's single configured boundary.
async fn drain_connections<F>(
    handle: &ServerHandle,
    shutdown: &CancellationToken,
    boundary: F,
) -> DrainOutcome
where
    F: Future<Output = ()>,
{
    // Order matters: event streams end first, so the drain below is only ever
    // waiting on connections that have real work left.
    shutdown.cancel();
    handle.graceful_shutdown(None);
    observe_drain(handle, boundary).await
}

/// Watches the live connection count until `boundary` resolves.
///
/// Every count is sampled after its preceding wait. In particular, the final
/// count and the force-close action happen together at the boundary, so the
/// reported count is exactly the set whose survival caused JIT to invoke the
/// handle's immediate shutdown.
async fn observe_drain<F>(handle: &ServerHandle, boundary: F) -> DrainOutcome
where
    F: Future<Output = ()>,
{
    tokio::pin!(boundary);

    loop {
        let connections = handle.connection_count();
        if connections == 0 {
            return DrainOutcome::Drained;
        }

        tokio::select! {
            biased;
            () = &mut boundary => {
                let connections = handle.connection_count();
                if connections == 0 {
                    return DrainOutcome::Drained;
                }

                handle.shutdown();
                return DrainOutcome::ForcedClosed { connections };
            }
            () = sleep(DRAIN_SAMPLE_INTERVAL) => {}
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
    use tokio::sync::{mpsc, oneshot};
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

    /// Serves requests that report handler entry and then remain in flight.
    async fn serve_stalling_on_loopback(
        handle: &ServerHandle,
    ) -> (
        SocketAddr,
        JoinHandle<io::Result<()>>,
        mpsc::UnboundedReceiver<()>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        let (started, started_requests) = mpsc::unbounded_channel();
        let app = Router::new().route(
            "/stall",
            get(move || {
                let started = started.clone();
                async move {
                    started.send(()).expect("the test observes handler entry");
                    std::future::pending::<&'static str>().await
                }
            }),
        );
        let server = axum_server::from_tcp(listener)
            .expect("adopt the bound listener")
            .handle(handle.clone());
        let serving = tokio::spawn(server.serve(app.into_make_service()));
        let addr = timeout(TEST_BUDGET, handle.listening())
            .await
            .expect("the server binds within the test budget")
            .expect("the server reports its bound address");
        (addr, serving, started_requests)
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

    /// Opens a complete request whose handler remains in flight.
    async fn open_stalling_request(addr: SocketAddr) -> TcpStream {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream
            .write_all(b"GET /stall HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .expect("send a request to the stalling handler");
        stream.flush().await.expect("flush the stalling request");
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
    async fn test_drain_connections_force_closes_exact_survivors_when_graceful_notice_is_observed_late(
    ) {
        let handle = ServerHandle::new();
        let (addr, serving, mut started_requests) = serve_stalling_on_loopback(&handle).await;
        let mut first_stalled = open_stalling_request(addr).await;
        let mut second_stalled = open_stalling_request(addr).await;
        for _ in 0..2 {
            timeout(TEST_BUDGET, started_requests.recv())
                .await
                .expect("each stalling handler starts within the test budget")
                .expect("the handler-entry channel remains open");
        }
        wait_for_connection_count(&handle, 2).await;

        // Leave the accepted connection tasks alive while preventing the main
        // serving task from observing the graceful notification and beginning
        // axum-server's post-accept-loop drain. This deterministically models
        // the notification-to-deadline race: only JIT's boundary can force the
        // two connection tasks to stop.
        serving.abort();
        assert!(
            serving
                .await
                .expect_err("the serving task was aborted")
                .is_cancelled(),
            "the serving task must stop observing handle notifications"
        );
        let (expire, boundary) = oneshot::channel();
        let shutdown = CancellationToken::new();
        let observer = tokio::spawn({
            let handle = handle.clone();
            let shutdown = shutdown.clone();
            async move {
                drain_connections(&handle, &shutdown, async {
                    boundary
                        .await
                        .expect("the test triggers the drain boundary")
                })
                .await
            }
        });

        timeout(TEST_BUDGET, shutdown.cancelled())
            .await
            .expect("the drain starts within the test budget");
        expire
            .send(())
            .expect("the drain observer is waiting on the boundary");
        let outcome = timeout(TEST_BUDGET, observer)
            .await
            .expect("the drain observer finishes within the test budget")
            .expect("the drain observer does not panic");

        assert_eq!(outcome, DrainOutcome::ForcedClosed { connections: 2 });
        for stalled in [&mut first_stalled, &mut second_stalled] {
            let mut byte = [0u8; 1];
            let read = timeout(TEST_BUDGET, stalled.read(&mut byte))
                .await
                .expect("the configured boundary closes the stalled connection")
                .expect("read the closed connection");
            assert_eq!(read, 0, "every sampled survivor must be force-closed");
        }
    }

    #[tokio::test]
    async fn test_run_shutdown_sequence_reports_drained_when_final_interval_empties() {
        let handle = ServerHandle::new();
        let (addr, serving) = serve_on_loopback(&handle).await;
        let mut completing = open_stalled_connection(addr).await;
        wait_for_connection_count(&handle, 1).await;
        let drain_deadline = DRAIN_SAMPLE_INTERVAL.saturating_mul(4);
        let (signal_sent, signal_started) = oneshot::channel();
        let shutdown_task = tokio::spawn(run_shutdown_sequence(
            handle.clone(),
            CancellationToken::new(),
            drain_deadline,
            async move {
                signal_sent
                    .send(())
                    .expect("the test waits for the shutdown signal");
                Ok(ShutdownSignal::Terminate)
            },
        ));

        signal_started
            .await
            .expect("the shutdown sequence starts before the connection completes");
        // The connection remains live through the first three 100 ms samples,
        // then completes halfway through the final polling interval. The drain
        // outcome therefore selects the non-forced-close logging branch.
        sleep(DRAIN_SAMPLE_INTERVAL.saturating_mul(3) + DRAIN_SAMPLE_INTERVAL / 2).await;
        completing
            .write_all(b"\r\n")
            .await
            .expect("complete the request during the final polling interval");
        completing
            .flush()
            .await
            .expect("flush the completed request");

        let outcome = timeout(TEST_BUDGET, shutdown_task)
            .await
            .expect("the shutdown sequence completes within the test budget")
            .expect("the shutdown task does not panic")
            .expect("a delivered signal is not an error");
        assert_eq!(
            outcome,
            DrainOutcome::Drained,
            "a connection that drains in the final interval must not report a forced close"
        );
        timeout(TEST_BUDGET, serving)
            .await
            .expect("the serve future ends within the test budget")
            .expect("the serve task does not panic")
            .expect("the serve future returns Ok after the final-interval drain");
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
