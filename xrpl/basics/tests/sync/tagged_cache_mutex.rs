use basics::tagged_cache::{ManualClock, TaggedCache};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use time::Duration;

#[test]
fn tagged_cache_peek_mutex_exposes_recursive_same_thread_reentry() {
    let cache = TaggedCache::<u32, String, ManualClock>::new(
        "peek-mutex",
        4,
        Duration::seconds(1),
        ManualClock::new(0),
    );

    let _guard = cache
        .peek_mutex()
        .lock()
        .expect("tagged cache mutex should lock");
    let _nested = cache
        .peek_mutex()
        .try_lock()
        .expect("same thread should re-enter recursive tagged cache mutex");

    assert_eq!(cache.get_track_size(), 0);
    assert!(!cache.touch_if_exists(&1));
    assert!(cache.peek_mutex().try_lock().is_ok());
}

#[test]
fn tagged_cache_peek_mutex_supports_unlock_relock_workflow_unique_lock() {
    let cache = Arc::new(TaggedCache::<u32, String, ManualClock>::new(
        "peek-mutex-relock",
        4,
        Duration::seconds(1),
        ManualClock::new(0),
    ));
    let started = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));

    let mut lock = cache
        .peek_mutex()
        .unique_lock()
        .expect("tagged cache unique lock should lock");
    lock.unlock();
    assert!(!lock.is_locked());

    let join = thread::spawn({
        let cache = Arc::clone(&cache);
        let started = Arc::clone(&started);
        let finished = Arc::clone(&finished);
        move || {
            started.store(true, Ordering::SeqCst);
            assert!(!cache.insert(42, "forty-two".to_owned()));
            finished.store(true, Ordering::SeqCst);
        }
    });

    while !started.load(Ordering::SeqCst) || !finished.load(Ordering::SeqCst) {
        thread::yield_now();
    }

    lock.lock()
        .expect("tagged cache unique lock should relock after mutation");
    assert!(cache.touch_if_exists(&42));
    assert_eq!(cache.get_track_size(), 1);

    join.join().expect("thread join");
}

#[test]
fn tagged_cache_fetch_with_does_not_revive_weak_entry_found_after_handler_phase() {
    let clock = Arc::new(ManualClock::new(0));
    let cache = Arc::new(TaggedCache::<u32, String, _>::new(
        "fetch-with-weak-race",
        1,
        Duration::seconds(1),
        Arc::clone(&clock),
    ));
    let (handler_started_tx, handler_started_rx) = mpsc::channel();
    let (continue_handler_tx, continue_handler_rx) = mpsc::channel();

    let join = thread::spawn({
        let cache = Arc::clone(&cache);
        move || {
            cache.fetch_with(&7, || {
                handler_started_tx
                    .send(())
                    .expect("handler-started signal should send");
                continue_handler_rx
                    .recv()
                    .expect("continue signal should arrive");
                Some(Arc::new("new".to_owned()))
            })
        }
    });

    handler_started_rx
        .recv()
        .expect("handler should start before the race setup");

    assert!(!cache.insert(7, "existing".to_owned()));
    let kept = cache
        .fetch(&7)
        .expect("existing value should be retrievable");
    clock.advance_seconds(1);
    cache.sweep();
    assert_eq!(cache.get_cache_size(), 0);
    assert_eq!(cache.get_track_size(), 1);

    continue_handler_tx
        .send(())
        .expect("handler should be allowed to finish");

    let result = join.join().expect("thread join");
    assert!(result.is_none());

    let relocked = cache
        .fetch(&7)
        .expect("plain fetch should relock the weak entry");
    assert!(Arc::ptr_eq(&relocked, &kept));
    assert_eq!(relocked.as_str(), "existing");
}
