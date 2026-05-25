use basics::basic_config::{Section, get_if_exists, get_string};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerfLogSetup {
    pub perf_log: PathBuf,
    pub log_interval: Duration,
}

impl Default for PerfLogSetup {
    fn default() -> Self {
        Self {
            perf_log: PathBuf::new(),
            log_interval: Duration::from_secs(1),
        }
    }
}

pub fn setup_perf_log(section: &Section, config_dir: &Path) -> PerfLogSetup {
    let mut setup = PerfLogSetup::default();

    let perf_log = get_string(section, "perf_log", "");
    if !perf_log.is_empty() {
        setup.perf_log = resolve_relative_path(config_dir, PathBuf::from(perf_log));
    }

    let mut log_interval = 0_u64;
    if get_if_exists(section, "log_interval", &mut log_interval) {
        setup.log_interval = Duration::from_secs(log_interval);
    }

    setup
}

fn resolve_relative_path(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }

    let absolute_base = if base.is_absolute() {
        base.to_path_buf()
    } else {
        std::env::current_dir()
            .expect("current directory should be available for perf log setup")
            .join(base)
    };

    absolute_base.join(path)
}
