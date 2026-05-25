use protocol::{Rules, Ter};
use tx::{ApplyContext, ApplyFlags, PreclaimContext, PreflightContext};

#[test]
fn preflight_context_plain_and_batch_constructors_preserve_cpp_fields() {
    let rules = Rules::new(std::iter::empty());

    let plain = PreflightContext::<_, _, _, &str>::new(
        "registry",
        "tx",
        rules.clone(),
        ApplyFlags::NONE,
        "journal",
    );
    assert_eq!(plain.registry, "registry");
    assert_eq!(plain.tx, "tx");
    assert_eq!(plain.rules, rules.clone());
    assert_eq!(plain.flags, ApplyFlags::NONE);
    assert_eq!(plain.parent_batch_id, None);
    assert_eq!(plain.journal, "journal");

    let batch = PreflightContext::new_batch(
        "registry",
        "tx",
        "batch",
        rules.clone(),
        ApplyFlags::BATCH,
        "journal",
    );
    assert_eq!(batch.parent_batch_id, Some("batch"));
    assert_eq!(batch.flags, ApplyFlags::BATCH);
}

#[test]
fn preclaim_context_requires_parent_batch_id_when_batch_flag_is_set() {
    let plain = PreclaimContext::<_, _, _, _, &str>::new(
        "registry",
        "view",
        Ter::TES_SUCCESS,
        "tx",
        ApplyFlags::NONE,
        "journal",
    );
    assert_eq!(plain.parent_batch_id, None);
    assert_eq!(plain.preflight_result, Ter::TES_SUCCESS);

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
fn apply_context_exposes_private_base_and_working_view_accessors_shape() {
    let mut ctx = ApplyContext::<_, _, _, _, _, _, &str>::new_batch(
        "registry",
        String::from("base"),
        vec![1],
        "batch",
        "tx",
        Ter::TES_SUCCESS,
        10_u64,
        ApplyFlags::BATCH,
        "journal",
    );

    ctx.base_mut().push_str("-updated");
    ctx.view_mut().push(2);

    assert_eq!(ctx.registry, "registry");
    assert_eq!(ctx.tx, "tx");
    assert_eq!(ctx.preclaim_result, Ter::TES_SUCCESS);
    assert_eq!(ctx.base_fee, 10_u64);
    assert_eq!(ctx.parent_batch_id, Some("batch"));
    assert_eq!(ctx.base(), "base-updated");
    assert_eq!(ctx.view(), &vec![1, 2]);
    assert_eq!(ctx.flags(), ApplyFlags::BATCH);
    assert_eq!(ctx.journal, "journal");
}
