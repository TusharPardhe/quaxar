//! HashRouter configuration parser.
//!

use std::collections::HashMap;
use std::time::Duration;

/// Configuration for the HashRouter.
#[derive(Debug, Clone)]
pub struct HashRouterSetup {
    pub hold_time: Duration,
    pub relay_time: Duration,
}

impl Default for HashRouterSetup {
    fn default() -> Self {
        Self {
            hold_time: Duration::from_secs(300),
            relay_time: Duration::from_secs(60),
        }
    }
}

/// Parse [hashrouter] config section into HashRouterSetup.
///
pub fn setup_hash_router(config: &HashMap<String, String>) -> Result<HashRouterSetup, String> {
    let mut setup = HashRouterSetup::default();

    if let Some(val) = config.get("hold_time") {
        let tmp: i32 = val
            .parse()
            .map_err(|_| format!("invalid hold_time value: {}", val))?;
        if tmp < 12 {
            return Err("HashRouter hold time must be at least 12 seconds (the \
                 approximate validation time for three ledgers)."
                .into());
        }
        setup.hold_time = Duration::from_secs(tmp as u64);
    }

    if let Some(val) = config.get("relay_time") {
        let tmp: i32 = val
            .parse()
            .map_err(|_| format!("invalid relay_time value: {}", val))?;
        if tmp < 8 {
            return Err("HashRouter relay time must be at least 8 seconds (the \
                 approximate validation time for two ledgers)."
                .into());
        }
        setup.relay_time = Duration::from_secs(tmp as u64);
    }

    if setup.relay_time > setup.hold_time {
        return Err("HashRouter relay time must be less than or equal to hold time".into());
    }

    Ok(setup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let setup = setup_hash_router(&HashMap::new()).unwrap();
        assert_eq!(setup.hold_time, Duration::from_secs(300));
        assert_eq!(setup.relay_time, Duration::from_secs(60));
    }

    #[test]
    fn hold_time_too_low() {
        let mut c = HashMap::new();
        c.insert("hold_time".into(), "11".into());
        assert!(setup_hash_router(&c).unwrap_err().contains("at least 12"));
    }

    #[test]
    fn relay_time_too_low() {
        let mut c = HashMap::new();
        c.insert("relay_time".into(), "7".into());
        assert!(setup_hash_router(&c).unwrap_err().contains("at least 8"));
    }

    #[test]
    fn relay_exceeds_hold() {
        let mut c = HashMap::new();
        c.insert("hold_time".into(), "20".into());
        c.insert("relay_time".into(), "25".into());
        assert!(
            setup_hash_router(&c)
                .unwrap_err()
                .contains("less than or equal")
        );
    }

    #[test]
    fn valid_config() {
        let mut c = HashMap::new();
        c.insert("hold_time".into(), "60".into());
        c.insert("relay_time".into(), "30".into());
        let setup = setup_hash_router(&c).unwrap();
        assert_eq!(setup.hold_time, Duration::from_secs(60));
        assert_eq!(setup.relay_time, Duration::from_secs(30));
    }
}
