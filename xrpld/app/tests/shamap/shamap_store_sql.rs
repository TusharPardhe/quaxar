use app::{ClearSqlOutcome, clear_sql_batches};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn shamap_store_clear_sql_returns_immediately_for_empty_table() {
    let deletes = Arc::new(Mutex::new(Vec::new()));
    let sleeps = Arc::new(Mutex::new(Vec::<Duration>::new()));
    let stop_checks = Arc::new(Mutex::new(0usize));

    let outcome = clear_sql_batches(
        1_000,
        100,
        Duration::from_millis(25),
        || None,
        {
            let deletes = Arc::clone(&deletes);
            move |seq| {
                deletes
                    .lock()
                    .expect("deletes mutex must not be poisoned")
                    .push(seq)
            }
        },
        {
            let stop_checks = Arc::clone(&stop_checks);
            move || {
                *stop_checks.lock().expect("stop mutex must not be poisoned") += 1;
                false
            }
        },
        {
            let sleeps = Arc::clone(&sleeps);
            move |duration| {
                sleeps
                    .lock()
                    .expect("sleeps mutex must not be poisoned")
                    .push(duration)
            }
        },
    );

    assert_eq!(outcome, ClearSqlOutcome::Completed);
    assert!(
        deletes
            .lock()
            .expect("deletes mutex must not be poisoned")
            .is_empty()
    );
    assert!(
        sleeps
            .lock()
            .expect("sleeps mutex must not be poisoned")
            .is_empty()
    );
    assert_eq!(
        *stop_checks.lock().expect("stop mutex must not be poisoned"),
        0
    );
}

#[test]
fn shamap_store_clear_sql_returns_immediately_when_min_matches_last_rotated() {
    let deletes = Arc::new(Mutex::new(Vec::new()));
    let sleeps = Arc::new(Mutex::new(Vec::<Duration>::new()));
    let stop_checks = Arc::new(Mutex::new(0usize));

    let outcome = clear_sql_batches(
        1_000,
        100,
        Duration::from_millis(25),
        || Some(1_000),
        {
            let deletes = Arc::clone(&deletes);
            move |seq| {
                deletes
                    .lock()
                    .expect("deletes mutex must not be poisoned")
                    .push(seq)
            }
        },
        {
            let stop_checks = Arc::clone(&stop_checks);
            move || {
                *stop_checks.lock().expect("stop mutex must not be poisoned") += 1;
                false
            }
        },
        {
            let sleeps = Arc::clone(&sleeps);
            move |duration| {
                sleeps
                    .lock()
                    .expect("sleeps mutex must not be poisoned")
                    .push(duration)
            }
        },
    );

    assert_eq!(outcome, ClearSqlOutcome::Completed);
    assert!(
        deletes
            .lock()
            .expect("deletes mutex must not be poisoned")
            .is_empty()
    );
    assert!(
        sleeps
            .lock()
            .expect("sleeps mutex must not be poisoned")
            .is_empty()
    );
    assert_eq!(
        *stop_checks.lock().expect("stop mutex must not be poisoned"),
        1
    );
}

#[test]
fn shamap_store_clear_sql_deletes_in_batches_until_last_rotated() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sleeps = Arc::new(Mutex::new(Vec::<Duration>::new()));
    let stop_checks = Arc::new(Mutex::new(0usize));

    let outcome = clear_sql_batches(
        1_000,
        300,
        Duration::from_millis(25),
        || Some(100),
        {
            let events = Arc::clone(&events);
            move |seq| {
                events
                    .lock()
                    .expect("events mutex must not be poisoned")
                    .push(format!("delete:{seq}"));
            }
        },
        {
            let stop_checks = Arc::clone(&stop_checks);
            move || {
                *stop_checks.lock().expect("stop mutex must not be poisoned") += 1;
                false
            }
        },
        {
            let sleeps = Arc::clone(&sleeps);
            move |duration| {
                sleeps
                    .lock()
                    .expect("sleeps mutex must not be poisoned")
                    .push(duration)
            }
        },
    );

    assert_eq!(outcome, ClearSqlOutcome::Completed);
    assert_eq!(
        events
            .lock()
            .expect("events mutex must not be poisoned")
            .as_slice(),
        &[
            "delete:400".to_owned(),
            "delete:700".to_owned(),
            "delete:1000".to_owned()
        ]
    );
    assert_eq!(
        sleeps
            .lock()
            .expect("sleeps mutex must not be poisoned")
            .len(),
        2
    );
    assert_eq!(
        *stop_checks.lock().expect("stop mutex must not be poisoned"),
        6
    );
}

#[test]
fn shamap_store_clear_sql_stops_before_work_when_predicate_requests_it() {
    let deletes = Arc::new(Mutex::new(Vec::new()));
    let sleeps = Arc::new(Mutex::new(Vec::<Duration>::new()));
    let stop_checks = Arc::new(Mutex::new(0usize));

    let outcome = clear_sql_batches(
        1_000,
        300,
        Duration::from_millis(25),
        || Some(100),
        {
            let deletes = Arc::clone(&deletes);
            move |seq| {
                deletes
                    .lock()
                    .expect("deletes mutex must not be poisoned")
                    .push(seq)
            }
        },
        {
            let stop_checks = Arc::clone(&stop_checks);
            move || {
                let mut count = stop_checks.lock().expect("stop mutex must not be poisoned");
                *count += 1;
                true
            }
        },
        {
            let sleeps = Arc::clone(&sleeps);
            move |duration| {
                sleeps
                    .lock()
                    .expect("sleeps mutex must not be poisoned")
                    .push(duration)
            }
        },
    );

    assert_eq!(outcome, ClearSqlOutcome::Stopped);
    assert!(
        deletes
            .lock()
            .expect("deletes mutex must not be poisoned")
            .is_empty()
    );
    assert!(
        sleeps
            .lock()
            .expect("sleeps mutex must not be poisoned")
            .is_empty()
    );
    assert_eq!(
        *stop_checks.lock().expect("stop mutex must not be poisoned"),
        1
    );
}

#[test]
fn shamap_store_clear_sql_stops_after_batch_before_sleep() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sleeps = Arc::new(Mutex::new(Vec::<Duration>::new()));
    let stop_checks = Arc::new(Mutex::new(0usize));

    let outcome = clear_sql_batches(
        1_000,
        300,
        Duration::from_millis(25),
        || Some(100),
        {
            let events = Arc::clone(&events);
            move |seq| {
                events
                    .lock()
                    .expect("events mutex must not be poisoned")
                    .push(format!("delete:{seq}"));
            }
        },
        {
            let stop_checks = Arc::clone(&stop_checks);
            move || {
                let mut count = stop_checks.lock().expect("stop mutex must not be poisoned");
                *count += 1;
                *count >= 2
            }
        },
        {
            let sleeps = Arc::clone(&sleeps);
            move |duration| {
                sleeps
                    .lock()
                    .expect("sleeps mutex must not be poisoned")
                    .push(duration)
            }
        },
    );

    assert_eq!(outcome, ClearSqlOutcome::Stopped);
    assert_eq!(
        events
            .lock()
            .expect("events mutex must not be poisoned")
            .as_slice(),
        &["delete:400".to_owned()]
    );
    assert!(
        sleeps
            .lock()
            .expect("sleeps mutex must not be poisoned")
            .is_empty()
    );
    assert_eq!(
        *stop_checks.lock().expect("stop mutex must not be poisoned"),
        2
    );
}

#[test]
fn shamap_store_clear_sql_stops_after_sleep_before_next_batch() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sleeps = Arc::new(Mutex::new(Vec::<Duration>::new()));
    let stop_checks = Arc::new(Mutex::new(0usize));

    let outcome = clear_sql_batches(
        1_000,
        300,
        Duration::from_millis(25),
        || Some(100),
        {
            let events = Arc::clone(&events);
            move |seq| {
                events
                    .lock()
                    .expect("events mutex must not be poisoned")
                    .push(format!("delete:{seq}"));
            }
        },
        {
            let stop_checks = Arc::clone(&stop_checks);
            move || {
                let mut count = stop_checks.lock().expect("stop mutex must not be poisoned");
                *count += 1;
                *count >= 3
            }
        },
        {
            let sleeps = Arc::clone(&sleeps);
            move |duration| {
                sleeps
                    .lock()
                    .expect("sleeps mutex must not be poisoned")
                    .push(duration)
            }
        },
    );

    assert_eq!(outcome, ClearSqlOutcome::Stopped);
    assert_eq!(
        events
            .lock()
            .expect("events mutex must not be poisoned")
            .as_slice(),
        &["delete:400".to_owned()]
    );
    assert_eq!(
        sleeps
            .lock()
            .expect("sleeps mutex must not be poisoned")
            .len(),
        1
    );
    assert_eq!(
        *stop_checks.lock().expect("stop mutex must not be poisoned"),
        3
    );
}
