use std::process::Command;

fn main() {
    // Git commit hash
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=QUAXAR_GIT_COMMIT={}", commit.trim());

    // Full commit hash and branch for server_info `git` (rippled
    // xrpl::git::getCommitHash / getBuildBranch).
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    };
    // Re-run when HEAD moves (a new commit or a branch switch), so a build of
    // new code never reports the previous commit. Paths come from git so
    // worktrees resolve to their own HEAD.
    let head_ref = git(&["rev-parse", "--symbolic-full-name", "HEAD"]);
    let mut watched = vec![git(&["rev-parse", "--git-path", "HEAD"])];
    if head_ref.starts_with("refs/") {
        watched.push(git(&["rev-parse", "--git-path", &head_ref]));
    }
    watched.push(git(&["rev-parse", "--git-path", "packed-refs"]));
    // Declaring any rerun-if-changed drops cargo's default of re-running on
    // every package file change; keep that for the sources.
    watched.push("src".to_owned());
    for path in watched {
        if !path.is_empty() && std::path::Path::new(&path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    println!(
        "cargo:rustc-env=QUAXAR_GIT_HASH={}",
        git(&["rev-parse", "HEAD"])
    );
    println!(
        "cargo:rustc-env=QUAXAR_GIT_BRANCH={}",
        git(&["rev-parse", "--abbrev-ref", "HEAD"])
    );

    // Rustc version
    let rustc = Command::new("rustc")
        .args(["--version"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=QUAXAR_RUSTC_VERSION={}", rustc.trim());

    // Build date
    let date = Command::new("date")
        .args(["+%Y-%m-%d %H:%M:%S"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=QUAXAR_BUILD_DATE={}", date.trim());

    // Target triple
    println!(
        "cargo:rustc-env=TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".into())
    );
}
