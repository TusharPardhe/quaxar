use crate::shamap::shamap_store_component::SHAMapStoreComponentRuntime;
use crate::{
    SHAMapStore, SHAMapStoreCopyDisposition, SHAMapStoreHealthPolicy, SHAMapStoreHealthStatus,
    SHAMapStoreRunLoopStep, SHAMapStoreSavedState, SHAMapStoreSavedStateDb, runloop_step,
};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SHAMapStoreWorkerStep {
    pub runloop: SHAMapStoreRunLoopStep,
    pub rotated: bool,
    pub stopped: bool,
    pub minimum_online: Option<u32>,
}

pub fn run_shamap_store_worker_step(
    store: &mut SHAMapStore,
    runtime: &mut dyn SHAMapStoreComponentRuntime,
    state_db: Option<&Arc<SHAMapStoreSavedStateDb>>,
) -> Result<Option<SHAMapStoreWorkerStep>, String> {
    run_shamap_store_worker_step_with_policy_refresh(store, runtime, state_db, |_| {})
}

/// Run one maintenance step, refreshing live advisory-delete policy immediately
/// before the destructive boundary. A health failure or circuit-breaker expiry
/// abandons only this snapshot; `last_rotated` is never persisted until the
/// backend swap completes.
pub fn run_shamap_store_worker_step_with_policy_refresh<F>(
    store: &mut SHAMapStore,
    runtime: &mut dyn SHAMapStoreComponentRuntime,
    state_db: Option<&Arc<SHAMapStoreSavedStateDb>>,
    refresh_policy: F,
) -> Result<Option<SHAMapStoreWorkerStep>, String>
where
    F: FnMut(&mut SHAMapStore),
{
    run_shamap_store_worker_step_with_policy_refresh_and_stop(
        store,
        runtime,
        state_db,
        refresh_policy,
        || false,
    )
}

pub(crate) fn run_shamap_store_worker_step_with_policy_refresh_and_stop<F, S>(
    store: &mut SHAMapStore,
    runtime: &mut dyn SHAMapStoreComponentRuntime,
    state_db: Option<&Arc<SHAMapStoreSavedStateDb>>,
    mut refresh_policy: F,
    should_stop: S,
) -> Result<Option<SHAMapStoreWorkerStep>, String>
where
    F: FnMut(&mut SHAMapStore),
    S: Fn() -> bool,
{
    let Some(validated_ledger) = store.take_queued_ledger() else {
        return Ok(None);
    };
    let validated_seq = validated_ledger.header().seq;
    let previous_last_rotated = store.get_last_rotated();
    let health_policy = SHAMapStoreHealthPolicy {
        age_threshold: store.config().age_threshold,
        recovery_wait: store.config().recovery_wait,
    };
    let step = runloop_step(
        validated_seq,
        previous_last_rotated,
        store.delete_interval(),
        store.get_can_delete(),
        SHAMapStoreHealthStatus::KeepGoing,
    );

    if previous_last_rotated == 0 {
        let last_rotated = store.initialize_last_rotated(validated_seq);
        // The first subsequent eligible attempt establishes its own
        // last-success anchor; initialization is not a successful rotation
        // health check.
        store.set_online_delete_health_progress(validated_seq, 0);
        persist_last_rotated(state_db, last_rotated)?;
        return Ok(Some(finish_step(store, runtime, step, false, false)));
    }
    if !step.decision.ready_to_rotate {
        return Ok(Some(finish_step(store, runtime, step, false, false)));
    }

    let (last_good, _) = store.online_delete_health_progress();
    // Match rippled's SHAMapStoreImp exactly: lastGoodValidatedLedger_ is a
    // process-local health anchor and therefore starts at zero after restart.
    // The first healthWait() deliberately checks current mode/age without
    // requiring the in-memory complete-ledger range to extend back to the
    // durable rotation boundary. Once that check succeeds, the current
    // validated sequence becomes the anchor for every destructive-stage
    // checkpoint in this rotation.
    store.set_online_delete_health_progress(last_good, 0);
    match wait_for_health_or_stop(&health_policy, store, runtime, validated_seq, &should_stop) {
        SHAMapStoreHealthStatus::KeepGoing => {}
        SHAMapStoreHealthStatus::Stopping => {
            return Ok(Some(finish_step(store, runtime, step, false, true)));
        }
        SHAMapStoreHealthStatus::Expired | SHAMapStoreHealthStatus::Waiting(_) => {
            return Ok(Some(finish_step(store, runtime, step, false, false)));
        }
    }

    // Policy may have changed while recovery waited. Recheck it before any
    // destructive work begins.
    refresh_policy(store);
    if !store
        .rotation_decision(validated_seq, SHAMapStoreHealthStatus::KeepGoing)
        .ready_to_rotate
    {
        return Ok(Some(finish_step(store, runtime, step, false, false)));
    }

    if !checkpoint_allows(&health_policy, store, runtime, validated_seq, &should_stop) {
        let stopped = checkpoint_stopped(runtime, &should_stop);
        return Ok(Some(finish_step(store, runtime, step, false, stopped)));
    }

    // `clear_prior` deletes old ledger state, so its health check must be the
    // immediately preceding operation.
    store.note_rotation_boundary(previous_last_rotated);
    runtime.clear_prior(previous_last_rotated)?;

    // Copying a validated map writes into the rotating backend. Do not start
    // it from a stale health observation.
    if !checkpoint_allows(&health_policy, store, runtime, validated_seq, &should_stop) {
        let stopped = checkpoint_stopped(runtime, &should_stop);
        return Ok(Some(finish_step(store, runtime, step, false, stopped)));
    }
    match runtime.copy_validated_ledger(Arc::clone(&validated_ledger), health_policy)? {
        SHAMapStoreCopyDisposition::Completed { .. } => {}
        SHAMapStoreCopyDisposition::Stopped { .. } => {
            let stopped = checkpoint_stopped(runtime, &should_stop);
            return Ok(Some(finish_step(store, runtime, step, false, stopped)));
        }
        SHAMapStoreCopyDisposition::MissingNode { .. } => {
            return Ok(Some(finish_step(store, runtime, step, false, false)));
        }
    }

    // Do not open the archive-read exposure window once a fresh checkpoint
    // has rejected this snapshot.
    if !checkpoint_allows(&health_policy, store, runtime, validated_seq, &should_stop) {
        let stopped = checkpoint_stopped(runtime, &should_stop);
        return Ok(Some(finish_step(store, runtime, step, false, stopped)));
    }
    let _rotation_window = runtime.begin_rotation_window()?;

    // Cache freshening is another copy-to-writable stage.
    if !checkpoint_allows(&health_policy, store, runtime, validated_seq, &should_stop) {
        let stopped = checkpoint_stopped(runtime, &should_stop);
        return Ok(Some(finish_step(store, runtime, step, false, stopped)));
    }
    runtime.freshen_caches()?;

    if !checkpoint_allows(&health_policy, store, runtime, validated_seq, &should_stop) {
        let stopped = checkpoint_stopped(runtime, &should_stop);
        return Ok(Some(finish_step(store, runtime, step, false, stopped)));
    }
    runtime.prepare_rotation()?;

    // Clearing caches invalidates resident state, so check directly before it.
    if !checkpoint_allows(&health_policy, store, runtime, validated_seq, &should_stop) {
        let stopped = checkpoint_stopped(runtime, &should_stop);
        return Ok(Some(finish_step(store, runtime, step, false, stopped)));
    }
    runtime.clear_caches(validated_seq)?;

    // The backend swap changes the durable store topology. A disconnect or
    // gap after the cache clear abandons before that boundary advances.
    if !checkpoint_allows(&health_policy, store, runtime, validated_seq, &should_stop) {
        let stopped = checkpoint_stopped(runtime, &should_stop);
        return Ok(Some(finish_step(store, runtime, step, false, stopped)));
    }
    let (writable_db, archive_db) = runtime.rotate_backends()?;
    let next_state = SHAMapStoreSavedState {
        writable_db: if writable_db.is_empty() {
            store.saved_state().writable_db.clone()
        } else {
            writable_db
        },
        archive_db: if archive_db.is_empty() {
            store.saved_state().archive_db.clone()
        } else {
            archive_db
        },
        last_rotated: validated_seq,
    };
    persist_state(state_db, &next_state)?;
    store.set_saved_state(next_state);

    // This is deliberately a fresh health observation immediately before the
    // final cache clear. Once the backends are swapped and the state is
    // durable, that clear is the only allowed disconnected/gapped completion:
    // abandoning it would leave a completed rotation with stale caches.
    let _post_commit_health = health_policy.evaluate_with_progress(
        runtime,
        store.online_delete_health_progress().0,
        validated_seq,
    );
    runtime.clear_caches(validated_seq)?;
    Ok(Some(finish_step(store, runtime, step, true, false)))
}

fn checkpoint_allows<S>(
    policy: &SHAMapStoreHealthPolicy,
    store: &mut SHAMapStore,
    runtime: &mut dyn SHAMapStoreComponentRuntime,
    fallback_validated_seq: u32,
    should_stop: &S,
) -> bool
where
    S: Fn() -> bool,
{
    matches!(
        wait_for_health_or_stop(policy, store, runtime, fallback_validated_seq, should_stop),
        SHAMapStoreHealthStatus::KeepGoing
    )
}

fn checkpoint_stopped<S>(runtime: &dyn SHAMapStoreComponentRuntime, should_stop: &S) -> bool
where
    S: Fn() -> bool,
{
    should_stop() || runtime.is_stopping()
}

fn wait_for_health_or_stop<S>(
    policy: &SHAMapStoreHealthPolicy,
    store: &mut SHAMapStore,
    runtime: &mut dyn SHAMapStoreComponentRuntime,
    fallback_validated_seq: u32,
    should_stop: &S,
) -> SHAMapStoreHealthStatus
where
    S: Fn() -> bool,
{
    loop {
        if should_stop() {
            return SHAMapStoreHealthStatus::Stopping;
        }
        let (last_good, last_success) = store.online_delete_health_progress();
        let health = policy.evaluate_with_progress(runtime, last_good, fallback_validated_seq);
        match health.status {
            SHAMapStoreHealthStatus::Stopping => return SHAMapStoreHealthStatus::Stopping,
            SHAMapStoreHealthStatus::KeepGoing => {
                let last_success = if last_success == 0 {
                    health.validated_seq
                } else {
                    last_success
                };
                let circuit_breaker =
                    last_success.saturating_add(store.config().max_waiting_ledgers);
                if health.validated_seq >= circuit_breaker {
                    return SHAMapStoreHealthStatus::Expired;
                }
                // Do not advance last-good on a failed check: that is what
                // keeps an older gap blocking even as newer ledgers arrive.
                store.set_online_delete_health_progress(
                    last_good.max(health.validated_seq),
                    health.validated_seq,
                );
                return SHAMapStoreHealthStatus::KeepGoing;
            }
            SHAMapStoreHealthStatus::Waiting(duration) => {
                let last_success = if last_success == 0 {
                    health.validated_seq
                } else {
                    last_success
                };
                if health.validated_seq
                    >= last_success.saturating_add(store.config().max_waiting_ledgers)
                {
                    return SHAMapStoreHealthStatus::Expired;
                }
                store.set_online_delete_health_progress(last_good, last_success);
                runtime.sleep(duration);
            }
            SHAMapStoreHealthStatus::Expired => return SHAMapStoreHealthStatus::Expired,
        }
    }
}

fn finish_step(
    store: &mut SHAMapStore,
    runtime: &dyn SHAMapStoreComponentRuntime,
    step: SHAMapStoreRunLoopStep,
    rotated: bool,
    stopped: bool,
) -> SHAMapStoreWorkerStep {
    store.finish_rendezvous();
    SHAMapStoreWorkerStep {
        runloop: step,
        rotated,
        stopped,
        minimum_online: store.minimum_online(runtime),
    }
}

fn persist_last_rotated(
    state_db: Option<&Arc<SHAMapStoreSavedStateDb>>,
    seq: u32,
) -> Result<(), String> {
    if let Some(state_db) = state_db {
        state_db.set_last_rotated(seq)?;
    }
    Ok(())
}

fn persist_state(
    state_db: Option<&Arc<SHAMapStoreSavedStateDb>>,
    state: &SHAMapStoreSavedState,
) -> Result<(), String> {
    if let Some(state_db) = state_db {
        state_db.set_state(state)?;
    }
    Ok(())
}
