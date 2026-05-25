//! Current Rust transaction-context carriers mirrored from `xrpl/tx/Transactor.h`.
//!
//! These types keep the the reference implementation batch-flag and parent-batch invariants
//! explicit in Rust.

use protocol::{NotTec, Rules, Ter};

use crate::ApplyFlags;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightContext<Registry, Tx, Journal, ParentBatchId> {
    pub registry: Registry,
    pub tx: Tx,
    pub rules: Rules,
    pub flags: ApplyFlags,
    pub parent_batch_id: Option<ParentBatchId>,
    pub journal: Journal,
}

impl<Registry, Tx, Journal, ParentBatchId> PreflightContext<Registry, Tx, Journal, ParentBatchId> {
    pub fn new(
        registry: Registry,
        tx: Tx,
        rules: Rules,
        flags: ApplyFlags,
        journal: Journal,
    ) -> Self {
        assert_eq!(
            flags & ApplyFlags::BATCH,
            ApplyFlags::NONE,
            "Batch apply flag should not be set"
        );

        Self {
            registry,
            tx,
            rules,
            flags,
            parent_batch_id: None,
            journal,
        }
    }

    pub fn new_batch(
        registry: Registry,
        tx: Tx,
        parent_batch_id: ParentBatchId,
        rules: Rules,
        flags: ApplyFlags,
        journal: Journal,
    ) -> Self {
        assert_eq!(
            flags & ApplyFlags::BATCH,
            ApplyFlags::BATCH,
            "Batch apply flag should be set"
        );

        Self {
            registry,
            tx,
            rules,
            flags,
            parent_batch_id: Some(parent_batch_id),
            journal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreclaimContext<Registry, View, Tx, Journal, ParentBatchId> {
    pub registry: Registry,
    pub view: View,
    pub preflight_result: NotTec,
    pub flags: ApplyFlags,
    pub tx: Tx,
    pub parent_batch_id: Option<ParentBatchId>,
    pub journal: Journal,
}

impl<Registry, View, Tx, Journal, ParentBatchId>
    PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>
{
    pub fn new(
        registry: Registry,
        view: View,
        preflight_result: NotTec,
        tx: Tx,
        flags: ApplyFlags,
        journal: Journal,
    ) -> Self {
        Self::new_with_parent_batch_id(registry, view, preflight_result, tx, flags, None, journal)
    }

    pub fn new_batch(
        registry: Registry,
        view: View,
        preflight_result: NotTec,
        tx: Tx,
        flags: ApplyFlags,
        parent_batch_id: ParentBatchId,
        journal: Journal,
    ) -> Self {
        Self::new_with_parent_batch_id(
            registry,
            view,
            preflight_result,
            tx,
            flags,
            Some(parent_batch_id),
            journal,
        )
    }

    pub fn new_with_parent_batch_id(
        registry: Registry,
        view: View,
        preflight_result: NotTec,
        tx: Tx,
        flags: ApplyFlags,
        parent_batch_id: Option<ParentBatchId>,
        journal: Journal,
    ) -> Self {
        assert_eq!(
            parent_batch_id.is_some(),
            (flags & ApplyFlags::BATCH) == ApplyFlags::BATCH,
            "Parent Batch ID should be set if batch apply flag is set"
        );

        Self {
            registry,
            view,
            preflight_result,
            flags,
            tx,
            parent_batch_id,
            journal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId> {
    pub registry: Registry,
    pub tx: Tx,
    pub preclaim_result: Ter,
    pub base_fee: Fee,
    pub journal: Journal,
    pub parent_batch_id: Option<ParentBatchId>,
    base: BaseView,
    flags: ApplyFlags,
    view: View,
}

impl<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>
    ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>
{
    pub fn new(
        registry: Registry,
        base: BaseView,
        view: View,
        tx: Tx,
        preclaim_result: Ter,
        base_fee: Fee,
        flags: ApplyFlags,
        journal: Journal,
    ) -> Self {
        assert_eq!(
            flags & ApplyFlags::BATCH,
            ApplyFlags::NONE,
            "Batch apply flag should not be set"
        );

        Self {
            registry,
            tx,
            preclaim_result,
            base_fee,
            journal,
            parent_batch_id: None,
            base,
            flags,
            view,
        }
    }

    pub fn new_batch(
        registry: Registry,
        base: BaseView,
        view: View,
        parent_batch_id: ParentBatchId,
        tx: Tx,
        preclaim_result: Ter,
        base_fee: Fee,
        flags: ApplyFlags,
        journal: Journal,
    ) -> Self {
        assert_eq!(
            flags & ApplyFlags::BATCH,
            ApplyFlags::BATCH,
            "Batch apply flag should be set"
        );

        Self {
            registry,
            tx,
            preclaim_result,
            base_fee,
            journal,
            parent_batch_id: Some(parent_batch_id),
            base,
            flags,
            view,
        }
    }

    pub fn base(&self) -> &BaseView {
        &self.base
    }

    pub fn base_mut(&mut self) -> &mut BaseView {
        &mut self.base
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn view_mut(&mut self) -> &mut View {
        &mut self.view
    }

    pub fn flags(&self) -> ApplyFlags {
        self.flags
    }
}

#[cfg(test)]
mod tests {
    use super::{ApplyContext, PreclaimContext, PreflightContext};
    use crate::ApplyFlags;
    use protocol::{Rules, Ter};

    #[test]
    fn preflight_context_enforces_current_cpp_batch_flag_invariants() {
        let rules = Rules::new(std::iter::empty());

        let plain = PreflightContext::<_, _, _, &str>::new(
            "registry",
            "tx",
            rules.clone(),
            ApplyFlags::NONE,
            "journal",
        );
        assert_eq!(plain.parent_batch_id, None);

        let batch = PreflightContext::new_batch(
            "registry",
            "tx",
            "batch",
            rules,
            ApplyFlags::BATCH,
            "journal",
        );
        assert_eq!(batch.parent_batch_id, Some("batch"));
    }

    #[test]
    #[should_panic(expected = "Batch apply flag should not be set")]
    fn preflight_context_plain_panics_when_batch_flag_is_set() {
        let _ = PreflightContext::<_, _, _, &str>::new(
            "registry",
            "tx",
            Rules::new(std::iter::empty()),
            ApplyFlags::BATCH,
            "journal",
        );
    }

    #[test]
    fn preclaim_context_enforces_parent_batch_match() {
        let plain = PreclaimContext::<_, _, _, _, &str>::new(
            "registry",
            "view",
            Ter::TES_SUCCESS,
            "tx",
            ApplyFlags::NONE,
            "journal",
        );
        assert_eq!(plain.parent_batch_id, None);

        let batch = PreclaimContext::new_batch(
            "registry",
            "view",
            Ter::TES_SUCCESS,
            "tx",
            ApplyFlags::BATCH,
            "batch",
            "journal",
        );
        assert_eq!(batch.parent_batch_id, Some("batch"));
    }

    #[test]
    #[should_panic(expected = "Parent Batch ID should be set if batch apply flag is set")]
    fn preclaim_context_panics_when_parent_batch_and_flag_disagree() {
        let _ = PreclaimContext::<_, _, _, _, &str>::new_with_parent_batch_id(
            "registry",
            "view",
            Ter::TES_SUCCESS,
            "tx",
            ApplyFlags::BATCH,
            None,
            "journal",
        );
    }

    #[test]
    fn apply_context_preserves_private_base_view_and_flags() {
        let mut ctx = ApplyContext::<_, _, _, _, _, _, &str>::new(
            "registry",
            String::from("base"),
            Vec::<i32>::new(),
            "tx",
            Ter::TES_SUCCESS,
            10_u64,
            ApplyFlags::RETRY,
            "journal",
        );

        ctx.base_mut().push_str("-updated");
        ctx.view_mut().push(1);

        assert_eq!(ctx.base(), "base-updated");
        assert_eq!(ctx.view(), &vec![1]);
        assert_eq!(ctx.flags(), ApplyFlags::RETRY);
        assert_eq!(ctx.parent_batch_id, None);
    }

    #[test]
    #[should_panic(expected = "Batch apply flag should be set")]
    fn apply_context_batch_constructor_requires_batch_flag() {
        let _ = ApplyContext::<_, _, _, _, _, _, &str>::new_batch(
            "registry",
            "base",
            "view",
            "batch",
            "tx",
            Ter::TES_SUCCESS,
            10_u64,
            ApplyFlags::NONE,
            "journal",
        );
    }
}
