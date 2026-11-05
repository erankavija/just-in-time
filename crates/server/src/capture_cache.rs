use crate::watcher::ChangeTracker;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;

type Generation<T> = (u64, Arc<OnceCell<Arc<T>>>);

pub(crate) struct CaptureCache<T> {
    current: Mutex<Option<Generation<T>>>,
}

impl<T> Default for CaptureCache<T> {
    fn default() -> Self {
        Self {
            current: Mutex::new(None),
        }
    }
}

impl<T> CaptureCache<T> {
    pub(crate) async fn get<E, F, Fut>(
        &self,
        tracker: &ChangeTracker,
        capture: F,
    ) -> Result<Arc<T>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let cell = {
            let mut current = self
                .current
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let version = tracker.current_version();
            let entry = current.get_or_insert_with(|| (version, Arc::new(OnceCell::new())));
            if entry.0 != version {
                *entry = (version, Arc::new(OnceCell::new()));
            }
            Arc::clone(&entry.1)
        };
        cell.get_or_try_init(|| async { capture().await.map(Arc::new) })
            .await
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    #[tokio::test]
    async fn test_capture_shares_inflight_work_for_one_version() {
        let cache = Arc::new(CaptureCache::<usize>::default());
        let tracker = Arc::new(ChangeTracker::new(8));
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let first = tokio::spawn({
            let (cache, tracker, calls, started, release) = (
                cache.clone(),
                tracker.clone(),
                calls.clone(),
                started.clone(),
                release.clone(),
            );
            async move {
                cache
                    .get(&tracker, || async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        started.notify_one();
                        release.notified().await;
                        Ok::<_, ()>(7)
                    })
                    .await
                    .unwrap()
            }
        });
        started.notified().await;
        let second = tokio::spawn({
            let (cache, tracker, calls) = (cache.clone(), tracker.clone(), calls.clone());
            async move {
                cache
                    .get(&tracker, || async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, ()>(8)
                    })
                    .await
                    .unwrap()
            }
        });
        tokio::task::yield_now().await;
        release.notify_one();
        assert_eq!((*first.await.unwrap(), *second.await.unwrap()), (7, 7));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_capture_failure_is_retryable() {
        let cache = CaptureCache::<usize>::default();
        let tracker = ChangeTracker::new(8);
        assert!(cache
            .get(&tracker, || async { Err::<usize, _>(()) })
            .await
            .is_err());
        assert_eq!(
            *cache
                .get(&tracker, || async { Ok::<_, ()>(3) })
                .await
                .unwrap(),
            3
        );
    }

    #[tokio::test]
    async fn test_capture_old_completion_does_not_replace_new_generation() {
        let cache = Arc::new(CaptureCache::<usize>::default());
        let tracker = Arc::new(ChangeTracker::new(8));
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let old = tokio::spawn({
            let (cache, tracker, started, release) = (
                cache.clone(),
                tracker.clone(),
                started.clone(),
                release.clone(),
            );
            async move {
                cache
                    .get(&tracker, || async {
                        started.notify_one();
                        release.notified().await;
                        Ok::<_, ()>(1)
                    })
                    .await
                    .unwrap()
            }
        });
        started.notified().await;
        tracker.notify_change();
        assert_eq!(
            *cache
                .get(&tracker, || async { Ok::<_, ()>(2) })
                .await
                .unwrap(),
            2
        );
        release.notify_one();
        assert_eq!(*old.await.unwrap(), 1);
        assert_eq!(
            *cache
                .get(&tracker, || async { Ok::<_, ()>(3) })
                .await
                .unwrap(),
            2
        );
    }
}
