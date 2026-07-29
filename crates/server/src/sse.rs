//! Server-Sent Events endpoint for live change notifications.

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::watcher::ChangeTracker;

/// Create an SSE stream from a ChangeTracker.
///
/// Each change event sends `event: change` with `data: {"version": N}`.
///
/// The stream ends when `shutdown` is cancelled. Without that, a subscriber
/// with no pending changes would sit behind the keepalive below and hold its
/// connection open for the whole shutdown drain
/// (`crate::shutdown::GRACEFUL_DRAIN_TIMEOUT`).
pub fn change_stream(
    tracker: &ChangeTracker,
    shutdown: CancellationToken,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    Sse::new(change_events(tracker, shutdown)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

/// The change events themselves, before they are wrapped in an SSE response.
fn change_events(
    tracker: &ChangeTracker,
    shutdown: CancellationToken,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let rx = tracker.subscribe();
    let changes = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().map(|version| {
            Ok(Event::default()
                .event("change")
                .data(format!(r#"{{"version":{version}}}"#)))
        })
    });
    // Qualified: `tokio_stream::StreamExt` (used for `filter_map` above) has no
    // future-terminated adapter, and importing both extension traits would make
    // `filter_map` ambiguous.
    futures::StreamExt::take_until(changes, async move { shutdown.cancelled().await })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration as TokioDuration};
    use tokio_util::sync::CancellationToken;

    /// Bound on every wait a failing test would otherwise spend hanging.
    const TEST_BUDGET: TokioDuration = TokioDuration::from_secs(5);

    #[tokio::test]
    async fn test_change_events_yields_one_event_per_tracked_change() {
        let tracker = ChangeTracker::new(16);
        let mut events = Box::pin(change_events(&tracker, CancellationToken::new()));
        tracker.notify_change();
        tracker.notify_change();

        for expected in 1..=2 {
            let event = timeout(TEST_BUDGET, events.next())
                .await
                .unwrap_or_else(|_| panic!("change {expected} reaches the stream"));
            assert!(event.is_some(), "change {expected} reaches the stream");
        }
    }

    #[tokio::test]
    async fn test_change_events_ends_when_the_shutdown_token_is_cancelled() {
        let tracker = ChangeTracker::new(16);
        let shutdown = CancellationToken::new();
        let mut events = Box::pin(change_events(&tracker, shutdown.clone()));

        shutdown.cancel();

        let terminated = timeout(TEST_BUDGET, events.next())
            .await
            .expect("a cancelled stream ends instead of waiting for the next change");
        assert!(
            terminated.is_none(),
            "the event stream must reach EOF so its keepalive stops holding the connection open"
        );
    }

    #[tokio::test]
    async fn test_change_events_ends_on_cancellation_while_changes_remain_pending() {
        let tracker = ChangeTracker::new(16);
        let shutdown = CancellationToken::new();
        let mut events = Box::pin(change_events(&tracker, shutdown.clone()));
        tracker.notify_change();
        shutdown.cancel();

        // Cancellation ends the stream; a buffered change never keeps it alive.
        let mut delivered = 0;
        while let Some(_event) = timeout(TEST_BUDGET, events.next())
            .await
            .expect("a cancelled stream ends within the test budget")
        {
            delivered += 1;
            assert!(delivered <= 1, "the stream outlived its cancellation");
        }
    }
}
