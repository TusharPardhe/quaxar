//! Validator squelch tracking aligned with `overlay/Squelch.h`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use protocol::PublicKey;

use crate::slot::{Clock, MAX_UNSQUELCH_EXPIRE_PEERS, MIN_UNSQUELCH_EXPIRE, SystemClock};

#[derive(Debug)]
pub struct Squelch {
    clock: Arc<dyn Clock>,
    squelched: HashMap<PublicKey, Duration>,
}

impl Default for Squelch {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock))
    }
}

impl Squelch {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            squelched: HashMap::new(),
        }
    }

    pub fn add_squelch(&mut self, validator: PublicKey, squelch_duration: Duration) -> bool {
        if squelch_duration >= MIN_UNSQUELCH_EXPIRE
            && squelch_duration <= MAX_UNSQUELCH_EXPIRE_PEERS
        {
            self.squelched
                .insert(validator, self.clock.now() + squelch_duration);
            return true;
        }

        self.remove_squelch(validator);
        false
    }

    pub fn remove_squelch(&mut self, validator: PublicKey) {
        self.squelched.remove(&validator);
    }

    pub fn expire_squelch(&mut self, validator: PublicKey) -> bool {
        let now = self.clock.now();
        match self.squelched.get(&validator).copied() {
            None => true,
            Some(expire_at) if expire_at > now => false,
            Some(_) => {
                self.squelched.remove(&validator);
                true
            }
        }
    }

    pub fn is_squelched(&mut self, validator: PublicKey) -> bool {
        !self.expire_squelch(validator)
    }

    pub fn expiration(&self, validator: PublicKey) -> Option<Duration> {
        self.squelched.get(&validator).copied()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use protocol::{KeyType, SecretKey, derive_public_key};

    use super::Squelch;
    use crate::slot::{MAX_UNSQUELCH_EXPIRE_PEERS, MIN_UNSQUELCH_EXPIRE, ManualClock};

    #[test]
    fn squelch_expires_and_rejects_invalid_durations() {
        let clock = Arc::new(ManualClock::new(Duration::from_secs(1_000)));
        let secret = SecretKey::from_bytes([7u8; 32]);
        let validator =
            derive_public_key(KeyType::Secp256k1, &secret).expect("validator public key");
        let mut squelch = Squelch::new(clock.clone());

        assert!(!squelch.add_squelch(validator, Duration::from_secs(1)));
        assert!(squelch.expiration(validator).is_none());

        assert!(squelch.add_squelch(validator, MIN_UNSQUELCH_EXPIRE));
        assert!(squelch.is_squelched(validator));

        clock.advance(MAX_UNSQUELCH_EXPIRE_PEERS + Duration::from_secs(1));
        assert!(!squelch.is_squelched(validator));
        assert!(squelch.expiration(validator).is_none());
    }
}
