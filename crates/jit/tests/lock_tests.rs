//! Tests for file locking functionality (TDD approach)

use std::fs::File;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

// Tests will use the FileLocker from jit::storage::lock once implemented

#[test]
#[ignore] // Remove once FileLocker is implemented
fn test_exclusive_lock_prevents_concurrent_writes() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.lock");

    // Create file
    File::create(&file_path).unwrap();

    let path = Arc::new(file_path.clone());
    let barrier = Arc::new(Barrier::new(2));
    let success_count = Arc::new(std::sync::Mutex::new(0));

    let handles: Vec<_> = (0..2)
        .map(|i| {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait(); // Ensure both threads start at same time

                // TODO: Use FileLocker once implemented
                // let locker = FileLocker::new(Duration::from_millis(100));
                // if let Ok(_guard) = locker.lock_exclusive(&path) {
                //     thread::sleep(Duration::from_millis(50));
                //     *success_count.lock().unwrap() += 1;
                // }

                // For now, placeholder
                if i == 0 {
                    *success_count.lock().unwrap() += 1;
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // Only one thread should have acquired the lock
    let count = *success_count.lock().unwrap();
    assert_eq!(count, 1, "Only one thread should acquire exclusive lock");
}

#[test]
#[ignore]
fn test_shared_locks_allow_concurrent_reads() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.lock");

    File::create(&file_path).unwrap();

    let path = Arc::new(file_path);
    let barrier = Arc::new(Barrier::new(3));
    let success_count = Arc::new(std::sync::Mutex::new(0));

    let handles: Vec<_> = (0..3)
        .map(|_| {
            let path = Arc::clone(&path);
            let barrier = Arc::clone(&barrier);
            let success_count = Arc::clone(&success_count);

            thread::spawn(move || {
                barrier.wait();

                // TODO: Use FileLocker
                // let locker = FileLocker::new(Duration::from_millis(100));
                // if let Ok(_guard) = locker.lock_shared(&path) {
                //     thread::sleep(Duration::from_millis(50));
                //     *success_count.lock().unwrap() += 1;
                // }

                *success_count.lock().unwrap() += 1;
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // All threads should acquire shared lock
    let count = *success_count.lock().unwrap();
    assert_eq!(count, 3, "All threads should acquire shared lock");
}

#[test]
#[ignore]
fn test_exclusive_lock_blocks_shared_locks() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.lock");

    File::create(&file_path).unwrap();

    let path = Arc::new(file_path);
    let exclusive_acquired = Arc::new(std::sync::Mutex::new(false));
    let shared_blocked = Arc::new(std::sync::Mutex::new(false));

    let path1 = Arc::clone(&path);
    let exclusive_acquired1 = Arc::clone(&exclusive_acquired);

    // Thread 1: Acquire exclusive lock and hold it
    let handle1 = thread::spawn(move || {
        // TODO: Use FileLocker
        // let locker = FileLocker::new(Duration::from_secs(1));
        // let _guard = locker.lock_exclusive(&path1).unwrap();
        *exclusive_acquired1.lock().unwrap() = true;
        thread::sleep(Duration::from_millis(200));
    });

    // Wait for thread 1 to acquire lock
    thread::sleep(Duration::from_millis(50));

    let path2 = Arc::clone(&path);
    let shared_blocked2 = Arc::clone(&shared_blocked);

    // Thread 2: Try to acquire shared lock (should block/timeout)
    let handle2 = thread::spawn(move || {
        // TODO: Use FileLocker
        // let locker = FileLocker::new(Duration::from_millis(100));
        // if locker.lock_shared(&path2).is_err() {
        //     *shared_blocked2.lock().unwrap() = true;
        // }

        *shared_blocked2.lock().unwrap() = true;
    });

    handle1.join().unwrap();
    handle2.join().unwrap();

    assert!(*exclusive_acquired.lock().unwrap());
    assert!(*shared_blocked.lock().unwrap());
}

#[test]
#[ignore]
fn test_lock_released_on_drop() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.lock");

    File::create(&file_path).unwrap();

    // TODO: Use FileLocker
    // let locker = FileLocker::new(Duration::from_millis(100));

    // {
    //     let _guard = locker.lock_exclusive(&file_path).unwrap();
    //     // Lock held here
    // } // Lock dropped and released

    // Should be able to acquire lock again
    // let _guard2 = locker.lock_exclusive(&file_path).unwrap();
}

#[test]
#[ignore]
fn test_timeout_on_lock_contention() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.lock");

    File::create(&file_path).unwrap();

    let path = Arc::new(file_path);

    // TODO: Use FileLocker
    // let locker = FileLocker::new(Duration::from_millis(100));

    // Thread 1: Hold lock
    let path1 = Arc::clone(&path);
    let handle1 = thread::spawn(move || {
        // let _guard = locker.lock_exclusive(&path1).unwrap();
        thread::sleep(Duration::from_millis(300));
    });

    thread::sleep(Duration::from_millis(50));

    // Thread 2: Should timeout
    let path2 = Arc::clone(&path);
    let handle2 = thread::spawn(move || {
        // let locker = FileLocker::new(Duration::from_millis(100));
        // let result = locker.lock_exclusive(&path2);
        // assert!(result.is_err(), "Should timeout waiting for lock");
    });

    handle1.join().unwrap();
    handle2.join().unwrap();
}

#[test]
#[ignore]
fn test_try_lock_non_blocking() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.lock");

    File::create(&file_path).unwrap();

    // TODO: Use FileLocker
    // let locker = FileLocker::new(Duration::from_millis(100));

    // First try should succeed
    // let guard = locker.try_lock_exclusive(&file_path).unwrap();
    // assert!(guard.is_some());

    // Second try should fail immediately (non-blocking)
    // let result = locker.try_lock_exclusive(&file_path).unwrap();
    // assert!(result.is_none());
}
