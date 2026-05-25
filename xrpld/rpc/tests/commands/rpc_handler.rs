//! Tests for rpc handler.

use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use protocol::JsonValue;
use rpc::{
    HandlerCondition, Role, RpcErrorCode, RpcRuntime, fill_handler, handler_specs,
    method_from_params, role_required,
};

fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[derive(Default)]
struct FakeRuntime {
    jobs: Cell<u32>,
    max_jobs: Cell<u32>,
    has_current_ledger: Cell<bool>,
    has_closed_ledger: Cell<bool>,
}

impl RpcRuntime for FakeRuntime {
    fn client_job_count(&self) -> u32 {
        self.jobs.get()
    }

    fn max_job_queue_clients(&self) -> u32 {
        self.max_jobs.get()
    }

    fn has_current_ledger(&self) -> bool {
        self.has_current_ledger.get()
    }

    fn has_closed_ledger(&self) -> bool {
        self.has_closed_ledger.get()
    }
}

#[test]
fn rpc_handler_requires_matching_command_and_method() {
    let runtime = FakeRuntime {
        max_jobs: Cell::new(100),
        has_current_ledger: Cell::new(true),
        has_closed_ledger: Cell::new(true),
        ..FakeRuntime::default()
    };

    let error = fill_handler(
        &object([
            ("command", JsonValue::String("ledger".to_owned())),
            ("method", JsonValue::String("account_tx".to_owned())),
        ]),
        Role::User,
        2,
        &runtime,
    )
    .expect_err("mismatched command/method should fail");

    assert_eq!(error.error_code(), Some(RpcErrorCode::UnknownCommand));
}

#[test]
fn rpc_handler_applies_load_and_condition_gates() {
    let runtime = FakeRuntime {
        jobs: Cell::new(51),
        max_jobs: Cell::new(50),
        has_current_ledger: Cell::new(true),
        has_closed_ledger: Cell::new(true),
    };
    let error = fill_handler(
        &object([("command", JsonValue::String("ledger".to_owned()))]),
        Role::User,
        2,
        &runtime,
    )
    .expect_err("busy runtime should reject limited callers");
    assert_eq!(error.error_code(), Some(RpcErrorCode::TooBusy));

    let runtime = FakeRuntime {
        jobs: Cell::new(0),
        max_jobs: Cell::new(50),
        has_current_ledger: Cell::new(false),
        has_closed_ledger: Cell::new(true),
    };
    let error = fill_handler(
        &object([("command", JsonValue::String("path_find".to_owned()))]),
        Role::User,
        2,
        &runtime,
    )
    .expect_err("missing current ledger should fail");
    assert_eq!(error.error_code(), Some(RpcErrorCode::NotSynced));

    let runtime = FakeRuntime {
        jobs: Cell::new(0),
        max_jobs: Cell::new(50),
        has_current_ledger: Cell::new(true),
        has_closed_ledger: Cell::new(false),
    };
    let error = fill_handler(
        &object([("command", JsonValue::String("ledger_closed".to_owned()))]),
        Role::User,
        2,
        &runtime,
    )
    .expect_err("missing closed ledger should fail");
    assert_eq!(error.error_code(), Some(RpcErrorCode::NotSynced));
}

#[test]
fn rpc_handler_registry_exposes_role_and_condition() {
    let runtime = FakeRuntime {
        max_jobs: Cell::new(50),
        has_current_ledger: Cell::new(true),
        has_closed_ledger: Cell::new(true),
        ..FakeRuntime::default()
    };

    let handler = fill_handler(
        &object([
            ("command", JsonValue::String("path_find".to_owned())),
            ("method", JsonValue::String("path_find".to_owned())),
        ]),
        Role::User,
        2,
        &runtime,
    )
    .expect("path_find should resolve");

    assert_eq!(handler.name, "path_find");
    assert_eq!(handler.required_role, Role::User);
    assert_eq!(handler.condition, HandlerCondition::NeedsCurrentLedger);
    assert_eq!(role_required(2, false, "fee"), Role::User);
    assert_eq!(role_required(2, false, "ledger_closed"), Role::User);
    assert_eq!(role_required(2, false, "ledger_current"), Role::User);
    assert_eq!(role_required(2, false, "path_find"), Role::User);
    assert_eq!(role_required(2, false, "ripple_path_find"), Role::User);
    assert_eq!(role_required(2, false, "server_definitions"), Role::User);
    assert_eq!(role_required(2, false, "unknown"), Role::Forbid);
    assert_eq!(
        method_from_params(&object([(
            "command",
            JsonValue::String("ledger".to_owned())
        )]))
        .expect("method"),
        "ledger"
    );

    let handler = fill_handler(
        &object([("command", JsonValue::String("ledger_closed".to_owned()))]),
        Role::User,
        2,
        &runtime,
    )
    .expect("ledger_closed should resolve");
    assert_eq!(handler.condition, HandlerCondition::NeedsClosedLedger);

    let handler = fill_handler(
        &object([(
            "command",
            JsonValue::String("server_definitions".to_owned()),
        )]),
        Role::User,
        2,
        &runtime,
    )
    .expect("server_definitions should resolve");
    assert_eq!(handler.required_role, Role::User);
    assert_eq!(handler.condition, HandlerCondition::None);

    let handler = fill_handler(
        &object([("command", JsonValue::String("ripple_path_find".to_owned()))]),
        Role::User,
        2,
        &runtime,
    )
    .expect("ripple_path_find should resolve");
    assert_eq!(handler.required_role, Role::User);
    assert_eq!(handler.condition, HandlerCondition::None);
}

#[test]
fn rpc_handler_registry_table_plus_expected_aliases() {
    let cpp = load_cpp_handler_table()
        .lines()
        .filter_map(|line| {
            let marker = ".name = \"";
            let start = line.find(marker)? + marker.len();
            let rest = &line[start..];
            let end = rest.find('"')?;
            Some(rest[..end].to_owned())
        })
        .collect::<BTreeSet<_>>();

    let rust = handler_specs()
        .iter()
        .map(|handler| handler.name.to_owned())
        .collect::<BTreeSet<_>>();

    let only_in_rust = rust.difference(&cpp).cloned().collect::<Vec<_>>();
    let only_in_cpp = cpp.difference(&rust).cloned().collect::<Vec<_>>();

    assert_eq!(only_in_cpp, Vec::<String>::new());
    assert_eq!(
        only_in_rust,
        vec![
            "ledger".to_owned(),
            "log_rotate".to_owned(),
            "no_ripple_check".to_owned()
        ]
    );
}

fn load_cpp_handler_table() -> String {
    fs::read_to_string(handler_table_path()).expect("Handler.cpp should be readable")
}

fn handler_table_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("XRPLD_CPP_REPO") {
        return PathBuf::from(explicit).join("src/xrpld/rpc/detail/Handler.cpp");
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .ancestors()
        .nth(2)
        .expect("rpc crate should live under the repo root");

    let mut candidates = Vec::new();
    candidates.push(repo_root.join("../xrpld/src/xrpld/rpc/detail/Handler.cpp"));
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("../xrpld/src/xrpld/rpc/detail/Handler.cpp"));
        candidates.extend(sibling_xrpld_candidates(&current_dir));
    }
    candidates.extend(sibling_xrpld_candidates(repo_root));
    candidates.extend(git_worktree_xrpld_candidates(repo_root));

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .expect("Handler.cpp should exist via XRPLD_CPP_REPO or ../xrpld")
}

fn sibling_xrpld_candidates(start: &Path) -> Vec<PathBuf> {
    start
        .ancestors()
        .map(|ancestor| ancestor.join("xrpld/src/xrpld/rpc/detail/Handler.cpp"))
        .collect()
}

fn git_worktree_xrpld_candidates(repo_root: &Path) -> Vec<PathBuf> {
    let Ok(output) = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_root)
        .output()
    else {
        return Vec::new();
    };

    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .map(|path| path.join("../xrpld/src/xrpld/rpc/detail/Handler.cpp"))
        .collect()
}
