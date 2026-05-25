use std::collections::BTreeMap;

use app::{StatusRpcGitInfo, StatusRpcSnapshot};
use basics::uptime_clock::UptimeClock;
use protocol::{JsonValue, get_version_string};

fn append_git_metadata(info: &mut BTreeMap<String, JsonValue>, git_info: &StatusRpcGitInfo) {
    let mut git = BTreeMap::new();
    if let Some(hash) = &git_info.hash {
        git.insert("hash".to_owned(), JsonValue::String(hash.clone()));
    }
    if let Some(branch) = &git_info.branch {
        git.insert("branch".to_owned(), JsonValue::String(branch.clone()));
    }
    if !git.is_empty() {
        info.insert("git".to_owned(), JsonValue::Object(git));
    }
}

fn non_empty_owned(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn compile_time_git_info() -> StatusRpcGitInfo {
    StatusRpcGitInfo {
        hash: non_empty_owned(
            option_env!("XRPL_GIT_COMMIT_HASH")
                .or(option_env!("GIT_COMMIT_HASH"))
                .or(option_env!("VERGEN_GIT_SHA")),
        ),
        branch: non_empty_owned(
            option_env!("XRPL_GIT_BUILD_BRANCH")
                .or(option_env!("GIT_BUILD_BRANCH"))
                .or(option_env!("VERGEN_GIT_BRANCH")),
        ),
    }
}

fn resolved_git_info(snapshot: &StatusRpcSnapshot) -> Option<StatusRpcGitInfo> {
    let snapshot_git = snapshot.git_info.as_ref();
    let StatusRpcGitInfo {
        hash: compile_time_hash,
        branch: compile_time_branch,
    } = compile_time_git_info();
    let git_info = StatusRpcGitInfo {
        hash: snapshot_git
            .and_then(|git| non_empty_owned(git.hash.as_deref()))
            .or(compile_time_hash),
        branch: snapshot_git
            .and_then(|git| non_empty_owned(git.branch.as_deref()))
            .or(compile_time_branch),
    };

    (git_info.hash.is_some() || git_info.branch.is_some()).then_some(git_info)
}

pub fn append_runtime_metadata(
    info: &mut BTreeMap<String, JsonValue>,
    snapshot: &StatusRpcSnapshot,
    human: bool,
    admin: bool,
) {
    info.insert(
        "build_version".to_owned(),
        JsonValue::String(get_version_string().to_owned()),
    );
    info.insert(
        "uptime".to_owned(),
        JsonValue::Unsigned(UptimeClock::now().as_seconds().max(0) as u64),
    );

    if human {
        if let Some(hostid) = &snapshot.hostid {
            info.insert("hostid".to_owned(), JsonValue::String(hostid.clone()));
        }
    }

    if let Some(server_domain) = &snapshot.server_domain {
        info.insert(
            "server_domain".to_owned(),
            JsonValue::String(server_domain.clone()),
        );
    }

    if let Some(io_latency_ms) = snapshot.io_latency_ms {
        info.insert(
            "io_latency_ms".to_owned(),
            JsonValue::Unsigned(io_latency_ms),
        );
    }

    if let Some(complete_ledgers) = &snapshot.complete_ledgers {
        info.insert(
            "complete_ledgers".to_owned(),
            JsonValue::String(complete_ledgers.clone()),
        );
    }

    if let Some(fetch_pack) = snapshot.fetch_pack.filter(|value| *value != 0) {
        info.insert(
            "fetch_pack".to_owned(),
            JsonValue::Unsigned(u64::from(fetch_pack)),
        );
    }

    if admin {
        if let Some(node_size) = &snapshot.node_size {
            info.insert("node_size".to_owned(), JsonValue::String(node_size.clone()));
        }

        if let Some(git_info) = resolved_git_info(snapshot) {
            append_git_metadata(info, &git_info);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{append_runtime_metadata, get_version_string};
    use app::{StatusRpcGitInfo, StatusRpcSnapshot};
    use protocol::JsonValue;
    use std::collections::BTreeMap;

    #[test]
    fn runtime_metadata_includes_build_version_uptime_and_status_fields() {
        let mut info = BTreeMap::new();
        append_runtime_metadata(
            &mut info,
            &StatusRpcSnapshot {
                complete_ledgers: Some("98-99".to_owned()),
                fetch_pack: Some(1),
                ..StatusRpcSnapshot::default()
            },
            true,
            false,
        );

        assert_eq!(
            info.get("build_version"),
            Some(&JsonValue::String(get_version_string().to_owned()))
        );
        assert!(matches!(info.get("uptime"), Some(JsonValue::Unsigned(_))));
        assert_eq!(
            info.get("complete_ledgers"),
            Some(&JsonValue::String("98-99".to_owned()))
        );
        assert_eq!(info.get("fetch_pack"), Some(&JsonValue::Unsigned(1)));
    }

    #[test]
    fn runtime_metadata_applies_human_and_admin_status_fields() {
        let mut info = BTreeMap::new();
        append_runtime_metadata(
            &mut info,
            &StatusRpcSnapshot {
                hostid: Some("host-1".to_owned()),
                server_domain: Some("example.com".to_owned()),
                node_size: Some("large".to_owned()),
                io_latency_ms: Some(19),
                complete_ledgers: Some("1-10".to_owned()),
                fetch_pack: Some(4),
                git_info: Some(StatusRpcGitInfo {
                    hash: Some("abc123".to_owned()),
                    branch: Some("main".to_owned()),
                }),
                ..StatusRpcSnapshot::default()
            },
            true,
            true,
        );

        assert_eq!(
            info.get("hostid"),
            Some(&JsonValue::String("host-1".to_owned()))
        );
        assert_eq!(
            info.get("server_domain"),
            Some(&JsonValue::String("example.com".to_owned()))
        );
        assert_eq!(
            info.get("node_size"),
            Some(&JsonValue::String("large".to_owned()))
        );
        assert_eq!(info.get("io_latency_ms"), Some(&JsonValue::Unsigned(19)));
        assert_eq!(
            info.get("complete_ledgers"),
            Some(&JsonValue::String("1-10".to_owned()))
        );
        assert_eq!(info.get("fetch_pack"), Some(&JsonValue::Unsigned(4)));
        let JsonValue::Object(git) = info.get("git").expect("git metadata must exist") else {
            panic!("git metadata must be object");
        };
        assert_eq!(
            git.get("hash"),
            Some(&JsonValue::String("abc123".to_owned()))
        );
        assert_eq!(
            git.get("branch"),
            Some(&JsonValue::String("main".to_owned()))
        );
    }

    #[test]
    fn runtime_metadata_omits_human_and_admin_only_fields_when_not_allowed() {
        let mut info = BTreeMap::new();
        append_runtime_metadata(
            &mut info,
            &StatusRpcSnapshot {
                hostid: Some("host-1".to_owned()),
                node_size: Some("large".to_owned()),
                git_info: Some(StatusRpcGitInfo {
                    hash: Some("abc123".to_owned()),
                    branch: None,
                }),
                ..StatusRpcSnapshot::default()
            },
            false,
            false,
        );

        assert!(!info.contains_key("hostid"));
        assert!(!info.contains_key("node_size"));
        assert!(!info.contains_key("git"));
    }

    #[test]
    fn runtime_metadata_filters_empty_snapshot_git_fields_and_keeps_non_empty_values() {
        let mut info = BTreeMap::new();
        append_runtime_metadata(
            &mut info,
            &StatusRpcSnapshot {
                git_info: Some(StatusRpcGitInfo {
                    hash: Some("  ".to_owned()),
                    branch: Some("main".to_owned()),
                }),
                ..StatusRpcSnapshot::default()
            },
            true,
            true,
        );

        let JsonValue::Object(git) = info.get("git").expect("git metadata must exist") else {
            panic!("git metadata must be object");
        };
        assert_eq!(git.get("hash"), None);
        assert_eq!(
            git.get("branch"),
            Some(&JsonValue::String("main".to_owned()))
        );
    }
}
