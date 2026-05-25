use basics::local_value::LocalValue;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
fn local_value_thread_default_isolated_per_thread_public_surface() {
    let value = Arc::new(LocalValue::new(-1));

    assert_eq!(value.get_cloned(), -1);

    let first = {
        let value = Arc::clone(&value);
        thread::spawn(move || {
            assert_eq!(value.get_cloned(), -1);
            value.set(-2);
            value.get_cloned()
        })
    }
    .join()
    .expect("thread should complete");

    let second = {
        let value = Arc::clone(&value);
        thread::spawn(move || value.get_cloned())
    }
    .join()
    .expect("thread should complete");

    assert_eq!(first, -2);
    assert_eq!(second, -1);
    assert_eq!(value.get_cloned(), -1);
}

#[test]
fn local_value_replace_matches_thread_local_assignment_style_updates() {
    let value = LocalValue::new(10);

    assert_eq!(value.replace(20), 10);
    assert_eq!(value.get_cloned(), 20);

    value.with_mut(|slot| *slot += 5);
    assert_eq!(value.get_cloned(), 25);
}

#[test]
fn local_value_borrow_guards_match_operator_like_access_shape() {
    let value = LocalValue::new(String::from("seed"));

    {
        let borrowed = value.borrow();
        assert_eq!(borrowed.as_ref(), "seed");
    }

    {
        let mut borrowed = value.borrow_mut();
        borrowed.push_str("-ctx");
        assert_eq!(borrowed.as_ref(), "seed-ctx");
        borrowed.as_mut().push_str("-more");
        assert_eq!(borrowed.as_ref(), "seed-ctx-more");
    }

    assert_eq!(value.get_cloned(), "seed-ctx-more");
}

#[test]
fn local_value_new_with_initializes_once_and_keeps_public_thread_local_values() {
    static INIT_COUNT: AtomicUsize = AtomicUsize::new(0);

    let value = Arc::new(LocalValue::new_with(|| {
        INIT_COUNT.fetch_add(1, Ordering::Relaxed);
        String::from("seed")
    }));

    assert_eq!(value.get_cloned(), "seed");
    value.set(String::from("main"));

    assert_eq!(value.get_cloned(), "main");
    let thread_seen = {
        let value = Arc::clone(&value);
        thread::spawn(move || {
            assert_eq!(value.get_cloned(), "seed");
            value.set(String::from("thread"));
            value.get_cloned()
        })
    }
    .join()
    .expect("thread should complete");

    assert_eq!(thread_seen, "thread");
    assert_eq!(value.get_cloned(), "main");
    assert_eq!(INIT_COUNT.load(Ordering::Relaxed), 1);
}

#[test]
fn nested_local_value_access_on_same_thread_does_not_deadlock() {
    let first = Arc::new(LocalValue::new(String::from("first")));
    let second = Arc::new(LocalValue::new(String::from("second")));
    let (done_tx, done_rx) = mpsc::channel();

    let first_for_thread = Arc::clone(&first);
    let second_for_thread = Arc::clone(&second);
    let worker = thread::spawn(move || {
        let borrowed = first_for_thread.borrow();
        let nested = second_for_thread.get_cloned();
        done_tx
            .send((borrowed.as_ref().clone(), nested))
            .expect("send nested access result");
    });

    let observed = done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("nested same-context access should complete");
    worker
        .join()
        .expect("nested local-value worker should finish");

    assert_eq!(observed.0, "first");
    assert_eq!(observed.1, "second");
}

#[test]
fn nested_same_local_value_access_on_same_thread_does_not_deadlock() {
    let value = Arc::new(LocalValue::new(String::from("seed")));
    let (done_tx, done_rx) = mpsc::channel();

    let value_for_thread = Arc::clone(&value);
    let worker = thread::spawn(move || {
        let borrowed = value_for_thread.borrow();
        let nested = value_for_thread.get_cloned();
        done_tx
            .send((borrowed.as_ref().clone(), nested))
            .expect("send nested same-value access result");
    });

    let observed = done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("nested same-value access should complete");
    worker
        .join()
        .expect("nested same-value local-value worker should finish");

    assert_eq!(observed.0, "seed");
    assert_eq!(observed.1, "seed");
}
