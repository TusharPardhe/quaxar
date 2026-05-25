//! the reference implementation compatibility surface.
//!
//! This ports the exact current deterministic wrapper behavior for:
//!
//! - the no-op `preflight(...)`,
//! - the `deleteSLE(...)` lookup wrapper,
//! - the lower loaded-`SLE` delete ordering,
//! - and the top-level `doApply()` delegation.

use protocol::{NotTec, Ter};

pub trait DidDeleteLoadedSleSink {
    fn dir_remove(&mut self) -> bool;
    fn owner_exists(&mut self) -> bool;
    fn adjust_owner_count(&mut self, delta: i32);
    fn update_owner(&mut self);
    fn erase_sle(&mut self);
}

pub trait DidDeleteApplySink {
    type OwnerNode: Copy;

    fn did_owner_node(&mut self) -> Option<Self::OwnerNode>;
    fn delete_loaded_sle(&mut self, owner_node: Self::OwnerNode) -> Ter;
}

pub fn run_did_delete_preflight() -> NotTec {
    Ter::TES_SUCCESS
}

pub fn run_did_delete_delete_loaded_sle<S: DidDeleteLoadedSleSink>(sink: &mut S) -> Ter {
    if !sink.dir_remove() {
        return Ter::TEF_BAD_LEDGER;
    }

    if !sink.owner_exists() {
        return Ter::TEC_INTERNAL;
    }

    sink.adjust_owner_count(-1);
    sink.update_owner();
    sink.erase_sle();
    Ter::TES_SUCCESS
}

pub fn run_did_delete_delete_sle<S: DidDeleteApplySink>(sink: &mut S) -> Ter {
    let Some(owner_node) = sink.did_owner_node() else {
        return Ter::TEC_NO_ENTRY;
    };

    sink.delete_loaded_sle(owner_node)
}

pub fn run_did_delete_do_apply<S: DidDeleteApplySink>(sink: &mut S) -> Ter {
    run_did_delete_delete_sle(sink)
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        DidDeleteApplySink, DidDeleteLoadedSleSink, run_did_delete_delete_loaded_sle,
        run_did_delete_delete_sle, run_did_delete_do_apply, run_did_delete_preflight,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestLoadedSleSink {
        dir_remove: bool,
        owner_exists: bool,
        events: Vec<String>,
        owner_count_delta: Vec<i32>,
        updated_owner: bool,
        erased_sle: bool,
    }

    impl TestLoadedSleSink {
        fn new() -> Self {
            Self {
                dir_remove: true,
                owner_exists: true,
                events: Vec::new(),
                owner_count_delta: Vec::new(),
                updated_owner: false,
                erased_sle: false,
            }
        }
    }

    impl DidDeleteLoadedSleSink for TestLoadedSleSink {
        fn dir_remove(&mut self) -> bool {
            self.events.push("dir_remove".to_string());
            self.dir_remove
        }

        fn owner_exists(&mut self) -> bool {
            self.events.push("owner_exists".to_string());
            self.owner_exists
        }

        fn adjust_owner_count(&mut self, delta: i32) {
            self.events.push(format!("adjust:{delta}"));
            self.owner_count_delta.push(delta);
        }

        fn update_owner(&mut self) {
            self.events.push("update_owner".to_string());
            self.updated_owner = true;
        }

        fn erase_sle(&mut self) {
            self.events.push("erase_sle".to_string());
            self.erased_sle = true;
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestApplySink {
        owner_node: Option<u64>,
        delete_result: Ter,
        delete_calls: Vec<u64>,
        events: Vec<String>,
    }

    impl TestApplySink {
        fn new() -> Self {
            Self {
                owner_node: Some(55),
                delete_result: Ter::TES_SUCCESS,
                delete_calls: Vec::new(),
                events: Vec::new(),
            }
        }
    }

    impl DidDeleteApplySink for TestApplySink {
        type OwnerNode = u64;

        fn did_owner_node(&mut self) -> Option<Self::OwnerNode> {
            self.events.push("peek_did".to_string());
            self.owner_node
        }

        fn delete_loaded_sle(&mut self, owner_node: Self::OwnerNode) -> Ter {
            self.events.push(format!("delete:{owner_node}"));
            self.delete_calls.push(owner_node);
            self.delete_result
        }
    }

    #[test]
    fn did_delete_preflight_is_noop() {
        assert_eq!(run_did_delete_preflight(), Ter::TES_SUCCESS);
    }

    #[test]
    fn did_delete_loaded_sle_maps_dir_remove_failure() {
        let mut sink = TestLoadedSleSink::new();
        sink.dir_remove = false;

        let result = run_did_delete_delete_loaded_sle(&mut sink);

        assert_eq!(result, Ter::TEF_BAD_LEDGER);
        assert_eq!(trans_token(result), "tefBAD_LEDGER");
        assert_eq!(sink.events, ["dir_remove"]);
    }

    #[test]
    fn did_delete_loaded_sle_maps_missing_owner() {
        let mut sink = TestLoadedSleSink::new();
        sink.owner_exists = false;

        let result = run_did_delete_delete_loaded_sle(&mut sink);

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(trans_token(result), "tecINTERNAL");
        assert_eq!(sink.events, ["dir_remove", "owner_exists"]);
    }

    #[test]
    fn did_delete_loaded_sle_preserves_current_order() {
        let mut sink = TestLoadedSleSink::new();

        let result = run_did_delete_delete_loaded_sle(&mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            sink.events,
            [
                "dir_remove",
                "owner_exists",
                "adjust:-1",
                "update_owner",
                "erase_sle"
            ]
        );
        assert_eq!(sink.owner_count_delta, vec![-1]);
        assert!(sink.updated_owner);
        assert!(sink.erased_sle);
    }

    #[test]
    fn did_delete_delete_sle_returns_no_entry_for_missing_did() {
        let mut sink = TestApplySink::new();
        sink.owner_node = None;

        let result = run_did_delete_delete_sle(&mut sink);

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
        assert_eq!(sink.events, ["peek_did"]);
    }

    #[test]
    fn did_delete_delete_sle_delegates_loaded_delete() {
        let mut sink = TestApplySink::new();
        sink.delete_result = Ter::TEC_INTERNAL;

        let result = run_did_delete_delete_sle(&mut sink);

        assert_eq!(result, Ter::TEC_INTERNAL);
        assert_eq!(sink.events, ["peek_did", "delete:55"]);
        assert_eq!(sink.delete_calls, vec![55]);
    }

    #[test]
    fn did_delete_do_apply_is_exact_delete_sle_wrapper() {
        let mut sink = TestApplySink::new();

        let result = run_did_delete_do_apply(&mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(sink.events, ["peek_did", "delete:55"]);
        assert_eq!(sink.delete_calls, vec![55]);
    }
}
