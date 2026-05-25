use crate::shamap::shamap_store_component::SHAMapStoreComponentRuntime;
use crate::shamap::shamap_store_health::wait_for_health;
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
        persist_last_rotated(state_db, last_rotated)?;
        store.finish_rendezvous();
        return Ok(Some(SHAMapStoreWorkerStep {
            runloop: step,
            rotated: false,
            stopped: false,
            minimum_online: store.minimum_online(runtime),
        }));
    }

    if !step.decision.ready_to_rotate {
        store.finish_rendezvous();
        return Ok(Some(SHAMapStoreWorkerStep {
            runloop: step,
            rotated: false,
            stopped: false,
            minimum_online: store.minimum_online(runtime),
        }));
    }

    if wait_for_health(&health_policy, runtime, |runtime, duration| {
        runtime.sleep(duration)
    }) == SHAMapStoreHealthStatus::Stopping
    {
        store.finish_rendezvous();
        return Ok(Some(SHAMapStoreWorkerStep {
            runloop: step,
            rotated: false,
            stopped: true,
            minimum_online: store.minimum_online(runtime),
        }));
    }

    store.note_rotation_boundary(previous_last_rotated);
    runtime.clear_prior(previous_last_rotated)?;
    if wait_for_health(&health_policy, runtime, |runtime, duration| {
        runtime.sleep(duration)
    }) == SHAMapStoreHealthStatus::Stopping
    {
        store.finish_rendezvous();
        return Ok(Some(SHAMapStoreWorkerStep {
            runloop: step,
            rotated: false,
            stopped: true,
            minimum_online: store.minimum_online(runtime),
        }));
    }

    match runtime.copy_validated_ledger(Arc::clone(&validated_ledger), health_policy)? {
        SHAMapStoreCopyDisposition::Completed { .. } => {}
        SHAMapStoreCopyDisposition::Stopped { .. } => {
            store.finish_rendezvous();
            return Ok(Some(SHAMapStoreWorkerStep {
                runloop: step,
                rotated: false,
                stopped: true,
                minimum_online: store.minimum_online(runtime),
            }));
        }
        SHAMapStoreCopyDisposition::MissingNode { .. } => {
            store.finish_rendezvous();
            return Ok(Some(SHAMapStoreWorkerStep {
                runloop: step,
                rotated: false,
                stopped: false,
                minimum_online: store.minimum_online(runtime),
            }));
        }
    }
    if wait_for_health(&health_policy, runtime, |runtime, duration| {
        runtime.sleep(duration)
    }) == SHAMapStoreHealthStatus::Stopping
    {
        store.finish_rendezvous();
        return Ok(Some(SHAMapStoreWorkerStep {
            runloop: step,
            rotated: false,
            stopped: true,
            minimum_online: store.minimum_online(runtime),
        }));
    }

    runtime.freshen_caches()?;
    if wait_for_health(&health_policy, runtime, |runtime, duration| {
        runtime.sleep(duration)
    }) == SHAMapStoreHealthStatus::Stopping
    {
        store.finish_rendezvous();
        return Ok(Some(SHAMapStoreWorkerStep {
            runloop: step,
            rotated: false,
            stopped: true,
            minimum_online: store.minimum_online(runtime),
        }));
    }

    runtime.prepare_rotation()?;
    runtime.clear_caches(validated_seq)?;
    if wait_for_health(&health_policy, runtime, |runtime, duration| {
        runtime.sleep(duration)
    }) == SHAMapStoreHealthStatus::Stopping
    {
        store.finish_rendezvous();
        return Ok(Some(SHAMapStoreWorkerStep {
            runloop: step,
            rotated: false,
            stopped: true,
            minimum_online: store.minimum_online(runtime),
        }));
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
    runtime.clear_caches(validated_seq)?;
    store.finish_rendezvous();

    Ok(Some(SHAMapStoreWorkerStep {
        runloop: step,
        rotated: true,
        stopped: false,
        minimum_online: store.minimum_online(runtime),
    }))
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
