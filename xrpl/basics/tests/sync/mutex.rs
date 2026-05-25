use basics::mutex::{ExclusiveLock, Mutex, RecursiveMutex, SharedLock, UniqueLock};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

#[test]
fn mutex_supports_explicit_lock_modes_without_breaking_existing_callers() {
    let mutex = Mutex::<String>::make_from("value".to_owned());
    assert_eq!(mutex.lock().get(), "value");
    assert_eq!(mutex.lock_shared().get(), "value");
}

#[test]
fn mutex_lock_with_exposes_cpp_style_lock_selection_and_guard_movement() {
    let mutex = Mutex::<i32, std::sync::RwLock<i32>>::new(41);

    {
        let shared = mutex.lock_with::<SharedLock>();
        assert_eq!(*shared, 41);
        let shared_guard = shared.into_guard();
        assert_eq!(*shared_guard, 41);
    }

    let exclusive = mutex.lock_with::<ExclusiveLock>();
    assert_eq!(*exclusive, 41);
    let exclusive_guard = exclusive.into_guard();
    assert_eq!(*exclusive_guard, 41);
}

#[test]
fn recursive_unique_lock_unlocks_and_relocks_through_one_movable_lock_object() {
    let mutex = Arc::new(RecursiveMutex::new(vec![1]));
    let ready = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));

    let mut lock = mutex.unique_lock().expect("initial lock");
    lock.push(2);
    lock.unlock();
    assert!(!lock.is_locked());

    let join = thread::spawn({
        let mutex = Arc::clone(&mutex);
        let ready = Arc::clone(&ready);
        let done = Arc::clone(&done);
        move || {
            let mut other = mutex.unique_lock().expect("other lock");
            ready.store(true, Ordering::SeqCst);
            other.push(3);
            done.store(true, Ordering::SeqCst);
        }
    });

    while !ready.load(Ordering::SeqCst) || !done.load(Ordering::SeqCst) {
        thread::yield_now();
    }

    lock.lock().expect("relock");
    assert!(lock.is_locked());
    lock.push(4);
    assert_eq!(&*lock, &[1, 2, 3, 4]);

    join.join().expect("thread join");
}

#[test]
fn recursive_unique_lock_try_lock_reports_blocked_owner_and_recovers_after_unlock() {
    let mutex = Arc::new(RecursiveMutex::new(7));
    let held = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));

    let mut lock = mutex.unique_lock().expect("initial lock");
    lock.unlock();

    let join = thread::spawn({
        let mutex = Arc::clone(&mutex);
        let held = Arc::clone(&held);
        let release = Arc::clone(&release);
        move || {
            let mut other = mutex.unique_lock().expect("other lock");
            held.store(true, Ordering::SeqCst);
            *other = 9;
            while !release.load(Ordering::SeqCst) {
                thread::yield_now();
            }
        }
    });

    while !held.load(Ordering::SeqCst) {
        thread::yield_now();
    }

    assert!(lock.try_lock().is_err());

    release.store(true, Ordering::SeqCst);
    join.join().expect("thread join");

    lock.try_lock().expect("reacquire after unlock");
    assert_eq!(*mutex.lock().expect("final lock"), 9);
}

#[test]
fn mutex_wrapper_exposes_recursive_unique_lock_mode_unique_lock() {
    let mutex = Arc::new(Mutex::<Vec<i32>, RecursiveMutex<Vec<i32>>>::new(vec![1]));
    let release = Arc::new(AtomicBool::new(false));
    let held = Arc::new(AtomicBool::new(false));

    let mut lock = mutex.lock_with::<UniqueLock>();
    lock.push(2);
    lock.guard_mut().unlock();
    assert!(!lock.guard().is_locked());

    let join = thread::spawn({
        let mutex = Arc::clone(&mutex);
        let release = Arc::clone(&release);
        let held = Arc::clone(&held);
        move || {
            let mut other = mutex.lock_with::<UniqueLock>();
            held.store(true, Ordering::SeqCst);
            other.push(3);
            while !release.load(Ordering::SeqCst) {
                thread::yield_now();
            }
        }
    });

    while !held.load(Ordering::SeqCst) {
        thread::yield_now();
    }

    release.store(true, Ordering::SeqCst);
    join.join().expect("thread join");

    lock.guard_mut().lock().expect("relock");
    lock.push(4);
    assert_eq!(&*lock, &[1, 2, 3, 4]);
}
