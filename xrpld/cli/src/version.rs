pub fn run() {
    println!();
    super::kv("Version", env!("CARGO_PKG_VERSION"));
    super::kv("Commit", env!("QUAXAR_GIT_COMMIT"));
    super::kv("Rustc", env!("QUAXAR_RUSTC_VERSION"));
    super::kv("Target", env!("TARGET"));
    super::kv(
        "Profile",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
    );
    super::kv("Built", env!("QUAXAR_BUILD_DATE"));
}
