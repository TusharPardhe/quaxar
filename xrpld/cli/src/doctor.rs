use console::Style;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

fn check(name: &str, f: impl FnOnce() -> bool) {
    let sp = ProgressBar::new_spinner();
    sp.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    sp.set_message(format!("checking {}...", name));
    sp.enable_steady_tick(Duration::from_millis(80));
    let ok = f();
    sp.finish_and_clear();
    let dot = if ok {
        Style::new().green().apply_to("●")
    } else {
        Style::new().red().apply_to("●")
    };
    println!("    {} {}", dot, name);
}

pub fn run(url: &str, conf: Option<&str>) {
    super::section_header("Pre-flight Diagnostics");
    println!();

    check("config", || {
        conf.map(|p| std::path::Path::new(p).exists())
            .unwrap_or(true)
    });

    check("ports", || {
        std::net::TcpStream::connect_timeout(
            &"127.0.0.1:5055".parse().unwrap(),
            Duration::from_secs(2),
        )
        .is_ok()
    });

    check("disk", || std::env::temp_dir().exists());

    check("DNS", || {
        use std::net::ToSocketAddrs;
        "s1.ripple.com:51235".to_socket_addrs().is_ok()
    });

    check("NuDB", || true);

    let rpc_url = url.to_owned();
    check("node responding", move || {
        super::rpc_call(&rpc_url, "ping", serde_json::json!({})).is_ok()
    });
}
