//! Server-Sent Events endpoint for live change notifications.

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::watcher::ChangeTracker;

/// Create an SSE stream from a ChangeTracker.
///
/// Each change event sends `event: change` with `data: {"version": N}`.
pub fn change_stream(
    tracker: &ChangeTracker,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = tracker.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().map(|version| {
            Ok(Event::default()
                .event("change")
                .data(format!(r#"{{"version":{version}}}"#)))
        })
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}
