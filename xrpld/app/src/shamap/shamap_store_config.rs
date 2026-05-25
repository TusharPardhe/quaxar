use basics::basic_config::{BasicConfig, Section};
use std::time::Duration;

const MINIMUM_DELETION_INTERVAL: u32 = 256;
const MINIMUM_DELETION_INTERVAL_STANDALONE: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SHAMapStoreConfig {
    pub delete_interval: u32,
    pub advisory_delete: bool,
    pub delete_batch: u32,
    pub back_off: Duration,
    pub age_threshold: Duration,
    pub recovery_wait: Duration,
}

impl Default for SHAMapStoreConfig {
    fn default() -> Self {
        Self {
            delete_interval: 0,
            advisory_delete: false,
            delete_batch: 100,
            back_off: Duration::from_millis(100),
            age_threshold: Duration::from_secs(60),
            recovery_wait: Duration::from_secs(5),
        }
    }
}

impl SHAMapStoreConfig {
    pub fn from_config(
        config: &BasicConfig,
        standalone: bool,
        ledger_history: u32,
    ) -> Result<Self, String> {
        let section = node_db_section(config)?;
        let delete_interval = read_u32(section, "online_delete").unwrap_or_default();
        let mut result = Self {
            delete_interval,
            ..Self::default()
        };

        if result.delete_interval == 0 {
            return Ok(result);
        }

        result.delete_batch = read_u32(section, "delete_batch").unwrap_or(result.delete_batch);
        if let Some(milliseconds) =
            read_u32(section, "back_off_milliseconds").or_else(|| read_u32(section, "backOff"))
        {
            result.back_off = Duration::from_millis(milliseconds as u64);
        }
        if let Some(seconds) = read_u32(section, "age_threshold_seconds") {
            result.age_threshold = Duration::from_secs(seconds as u64);
        }
        if let Some(seconds) = read_u32(section, "recovery_wait_seconds") {
            result.recovery_wait = Duration::from_secs(seconds as u64);
        }
        result.advisory_delete = read_bool(section, "advisory_delete").unwrap_or(false);

        let min_interval = if standalone {
            MINIMUM_DELETION_INTERVAL_STANDALONE
        } else {
            MINIMUM_DELETION_INTERVAL
        };
        if result.delete_interval < min_interval {
            return Err(format!("online_delete must be at least {min_interval}"));
        }
        if ledger_history > result.delete_interval {
            return Err(format!(
                "online_delete must not be less than ledger_history (currently {ledger_history})"
            ));
        }

        Ok(result)
    }
}

pub fn node_db_section(config: &BasicConfig) -> Result<&Section, String> {
    let section = config.section("node_db");
    if section.empty() {
        return Err("Missing [node_db] entry in configuration file".to_owned());
    }
    Ok(section)
}

fn read_u32(section: &Section, key: &str) -> Option<u32> {
    section.get::<u32>(key).ok().flatten()
}

fn read_bool(section: &Section, key: &str) -> Option<bool> {
    if let Some(value) = section.get::<bool>(key).ok().flatten() {
        return Some(value);
    }
    section
        .get::<i32>(key)
        .ok()
        .flatten()
        .map(|value| value != 0)
}

#[cfg(test)]
mod tests {
    use super::SHAMapStoreConfig;
    use basics::basic_config::BasicConfig;

    #[test]
    fn config_uses_cpp_online_delete_defaults() {
        let mut config = BasicConfig::new();
        let node_db = config.section_mut("node_db");
        node_db.set("type", "RocksDB");
        node_db.set("path", "/tmp/node_db");
        node_db.set("online_delete", "256");

        let parsed = SHAMapStoreConfig::from_config(&config, false, 128).expect("config");
        assert_eq!(parsed.delete_batch, 100);
        assert_eq!(parsed.back_off.as_millis(), 100);
        assert_eq!(parsed.age_threshold.as_secs(), 60);
        assert_eq!(parsed.recovery_wait.as_secs(), 5);
        assert!(!parsed.advisory_delete);
    }

    #[test]
    fn config_accepts_legacy_backoff_key_and_bool_like_advisory_delete() {
        let mut config = BasicConfig::new();
        let node_db = config.section_mut("node_db");
        node_db.set("type", "RocksDB");
        node_db.set("path", "/tmp/node_db");
        node_db.set("online_delete", "256");
        node_db.set("backOff", "250");
        node_db.set("advisory_delete", "1");

        let parsed = SHAMapStoreConfig::from_config(&config, false, 64).expect("config");
        assert_eq!(parsed.back_off.as_millis(), 250);
        assert!(parsed.advisory_delete);
    }

    #[test]
    fn config_enforces_cpp_online_delete_bounds() {
        let mut config = BasicConfig::new();
        config.section_mut("node_db").set("type", "RocksDB");
        config.section_mut("node_db").set("path", "/tmp/node_db");
        config.section_mut("node_db").set("online_delete", "128");
        let error =
            SHAMapStoreConfig::from_config(&config, false, 64).expect_err("interval should fail");
        assert_eq!(error, "online_delete must be at least 256");

        config.section_mut("node_db").set("online_delete", "256");
        let error =
            SHAMapStoreConfig::from_config(&config, false, 300).expect_err("history should fail");
        assert_eq!(
            error,
            "online_delete must not be less than ledger_history (currently 300)"
        );
    }
}
