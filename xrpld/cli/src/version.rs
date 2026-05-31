pub fn run() {
    println!();
    super::kv("Version", env!("CARGO_PKG_VERSION"));
    super::kv("Commit", env!("XRPLD_GIT_COMMIT"));
    super::kv("Rustc", env!("XRPLD_RUSTC_VERSION"));
    super::kv("Target", env!("TARGET"));
    super::kv(
        "Profile",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    );
    super::kv("Built", env!("XRPLD_BUILD_DATE"));
}
