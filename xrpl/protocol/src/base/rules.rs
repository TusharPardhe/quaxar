//! `xrpl/protocol/Rules.h` compatibility surface.
//!
//! This keeps the currently needed rule-set behavior in the protocol crate
//! instead of letting ledger own a protocol type.

use basics::base_uint::Uint256;
use basics::local_value::LocalValue;
use basics::number::{MantissaScale, set_mantissa_scale};
use basics::unordered_containers::{HardenedHashSet, HashSet};
use std::sync::OnceLock;

use crate::{feature_lending_protocol, feature_single_asset_vault};

#[derive(Debug, Clone, Default)]
pub struct Rules {
    presets: HashSet<Uint256>,
    amendments: HardenedHashSet<Uint256>,
    digest: Option<Uint256>,
}

impl Rules {
    pub fn new<I>(presets: I) -> Self
    where
        I: IntoIterator<Item = Uint256>,
    {
        Self {
            presets: presets.into_iter().collect(),
            amendments: HardenedHashSet::default(),
            digest: None,
        }
    }

    pub fn from_ledger<I, J>(presets: I, digest: Uint256, amendments: J) -> Self
    where
        I: IntoIterator<Item = Uint256>,
        J: IntoIterator<Item = Uint256>,
    {
        Self {
            presets: presets.into_iter().collect(),
            amendments: amendments.into_iter().collect(),
            digest: Some(digest),
        }
    }

    pub fn enabled(&self, feature: &Uint256) -> bool {
        self.presets.contains(feature) || self.amendments.contains(feature)
    }

    pub fn digest(&self) -> Option<Uint256> {
        self.digest
    }

    pub fn presets(&self) -> impl Iterator<Item = Uint256> + '_ {
        self.presets.iter().copied()
    }
}

fn current_transaction_rules_ref() -> &'static LocalValue<Option<Rules>> {
    static CURRENT_TRANSACTION_RULES: OnceLock<LocalValue<Option<Rules>>> = OnceLock::new();
    CURRENT_TRANSACTION_RULES.get_or_init(LocalValue::default)
}

pub fn get_current_transaction_rules() -> Option<Rules> {
    current_transaction_rules_ref().get_cloned()
}

pub fn set_current_transaction_rules(rules: Option<Rules>) {
    let enable_large_numbers = match &rules {
        None => true,
        Some(rules) => {
            rules.enabled(&feature_single_asset_vault())
                || rules.enabled(&feature_lending_protocol())
        }
    };

    set_mantissa_scale(if enable_large_numbers {
        MantissaScale::Large
    } else {
        MantissaScale::Small
    });
    current_transaction_rules_ref().set(rules);
}

pub fn is_feature_enabled(feature: &Uint256) -> bool {
    get_current_transaction_rules().is_some_and(|rules| rules.enabled(feature))
}

#[derive(Debug)]
pub struct CurrentTransactionRulesGuard {
    saved: Option<Rules>,
}

impl CurrentTransactionRulesGuard {
    pub fn new(rules: Rules) -> Self {
        let saved = get_current_transaction_rules();
        set_current_transaction_rules(Some(rules));
        Self { saved }
    }
}

impl Drop for CurrentTransactionRulesGuard {
    fn drop(&mut self) {
        set_current_transaction_rules(self.saved.take());
    }
}

pub fn make_rules_given_ledger<I, J>(
    presets: I,
    digest: Option<Uint256>,
    amendments: Option<J>,
) -> Rules
where
    I: IntoIterator<Item = Uint256>,
    J: IntoIterator<Item = Uint256>,
{
    match (digest, amendments) {
        (Some(digest), Some(amendments)) => Rules::from_ledger(presets, digest, amendments),
        _ => Rules::new(presets),
    }
}

pub fn make_rules_given_current<J>(
    current: &Rules,
    digest: Option<Uint256>,
    amendments: Option<J>,
) -> Rules
where
    J: IntoIterator<Item = Uint256>,
{
    make_rules_given_ledger(current.presets(), digest, amendments)
}

impl PartialEq for Rules {
    fn eq(&self, other: &Self) -> bool {
        if self.digest.is_none() && other.digest.is_none() {
            return true;
        }

        if self.digest.is_none() || other.digest.is_none() {
            return false;
        }

        debug_assert_eq!(
            self.presets, other.presets,
            "Rules equality assumes matching preset feature sets"
        );

        self.digest == other.digest
    }
}

impl Eq for Rules {}

#[cfg(test)]
mod tests {
    use super::{
        CurrentTransactionRulesGuard, Rules, get_current_transaction_rules, is_feature_enabled,
        make_rules_given_current, make_rules_given_ledger, set_current_transaction_rules,
    };
    use crate::{feature_lending_protocol, feature_single_asset_vault};
    use basics::base_uint::Uint256;
    use basics::local_value::{LocalSlotOwner, install_local_slot_owner};
    use basics::number::{MantissaScale, get_mantissa_scale};

    fn sample_uint256(fill: u8) -> Uint256 {
        Uint256::from_array([fill; 32])
    }

    fn with_local_slot_owner<R>(owner: &LocalSlotOwner, f: impl FnOnce() -> R) -> R {
        let _guard = install_local_slot_owner(owner);
        f()
    }

    #[test]
    fn enabled_checks_presets_and_ledger_amendments() {
        let preset = sample_uint256(0x11);
        let amendment = sample_uint256(0x22);
        let rules = Rules::from_ledger([preset], sample_uint256(0x33), [amendment]);

        assert!(rules.enabled(&preset));
        assert!(rules.enabled(&amendment));
        assert!(!rules.enabled(&sample_uint256(0x44)));
    }

    #[test]
    fn equality_matches_current_cpp_digest_semantics() {
        let preset = sample_uint256(0x51);
        let left = Rules::new([preset]);
        let right = Rules::new([preset]);
        let with_digest =
            Rules::from_ledger([preset], sample_uint256(0x52), [sample_uint256(0x53)]);
        let with_same_digest =
            Rules::from_ledger([preset], sample_uint256(0x52), [sample_uint256(0x54)]);
        let with_other_digest =
            Rules::from_ledger([preset], sample_uint256(0x55), [sample_uint256(0x53)]);

        assert_eq!(left, right);
        assert_eq!(with_digest, with_same_digest);
        assert_ne!(left, with_digest);
        assert_ne!(with_digest, with_other_digest);
    }

    #[test]
    fn make_rules_given_ledger_uses_digest_and_amendments_when_both_exist() {
        let preset = sample_uint256(0x61);
        let digest = sample_uint256(0x62);
        let amendment = sample_uint256(0x63);

        let rules = make_rules_given_ledger([preset], Some(digest), Some([amendment]));

        assert_eq!(rules, Rules::from_ledger([preset], digest, [amendment]));
        assert!(rules.enabled(&preset));
        assert!(rules.enabled(&amendment));
    }

    #[test]
    fn make_rules_given_ledger_falls_back_to_presets_when_digest_or_entry_is_missing() {
        let preset = sample_uint256(0x71);
        let amendment = sample_uint256(0x72);

        let missing_digest = make_rules_given_ledger([preset], None, Some([amendment]));
        let missing_entry =
            make_rules_given_ledger([preset], Some(sample_uint256(0x73)), None::<[Uint256; 1]>);

        assert_eq!(missing_digest, Rules::new([preset]));
        assert_eq!(missing_entry, Rules::new([preset]));
        assert!(!missing_digest.enabled(&amendment));
        assert!(!missing_entry.enabled(&amendment));
    }

    #[test]
    fn make_rules_given_current_reuses_current_presets() {
        let preset = sample_uint256(0x81);
        let digest = sample_uint256(0x82);
        let amendment = sample_uint256(0x83);
        let current = Rules::new([preset]);

        let rules = make_rules_given_current(&current, Some(digest), Some([amendment]));

        assert_eq!(rules, Rules::from_ledger([preset], digest, [amendment]));
        assert!(rules.enabled(&preset));
        assert!(rules.enabled(&amendment));
    }

    #[test]
    fn current_transaction_rules_default_to_none() {
        set_current_transaction_rules(None);
        assert_eq!(get_current_transaction_rules(), None);
        assert_eq!(get_mantissa_scale(), MantissaScale::Large);
        assert!(!is_feature_enabled(&sample_uint256(0x91)));
    }

    #[test]
    fn current_transaction_rules_round_trip_through_setter() {
        let preset = sample_uint256(0xA1);
        let amendment = sample_uint256(0xA2);
        let rules = Rules::from_ledger([preset], sample_uint256(0xA3), [amendment]);

        set_current_transaction_rules(Some(rules.clone()));

        assert_eq!(get_current_transaction_rules(), Some(rules));
        assert!(is_feature_enabled(&preset));
        assert!(is_feature_enabled(&amendment));

        set_current_transaction_rules(None);
    }

    #[test]
    fn current_transaction_rules_guard_restores_previous_value() {
        let original = Rules::new([sample_uint256(0xB1)]);
        let replacement = Rules::new([sample_uint256(0xB2)]);
        set_current_transaction_rules(Some(original.clone()));
        assert_eq!(get_mantissa_scale(), MantissaScale::Small);

        {
            let _guard = CurrentTransactionRulesGuard::new(replacement.clone());
            assert_eq!(get_current_transaction_rules(), Some(replacement));
            assert_eq!(get_mantissa_scale(), MantissaScale::Small);
        }

        assert_eq!(get_current_transaction_rules(), Some(original));
        assert_eq!(get_mantissa_scale(), MantissaScale::Small);
        set_current_transaction_rules(None);
    }

    #[test]
    fn current_transaction_rules_switch_mantissa_scale() {
        set_current_transaction_rules(Some(Rules::new([sample_uint256(0xD1)])));
        assert_eq!(get_mantissa_scale(), MantissaScale::Small);

        set_current_transaction_rules(Some(Rules::new([feature_single_asset_vault()])));
        assert_eq!(get_mantissa_scale(), MantissaScale::Large);

        set_current_transaction_rules(Some(Rules::new([feature_lending_protocol()])));
        assert_eq!(get_mantissa_scale(), MantissaScale::Large);

        set_current_transaction_rules(None);
        assert_eq!(get_mantissa_scale(), MantissaScale::Large);
    }

    #[test]
    fn current_transaction_rules_follow_explicit_local_context_scoping() {
        set_current_transaction_rules(Some(Rules::new([sample_uint256(0xC0)])));
        let feature = sample_uint256(0xC1);
        let rules = Rules::new([feature]);
        let owner = LocalSlotOwner::new();

        with_local_slot_owner(&owner, || {
            assert_eq!(get_current_transaction_rules(), None);
            assert_eq!(get_mantissa_scale(), MantissaScale::Small);
            set_current_transaction_rules(Some(rules.clone()));
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            assert!(is_feature_enabled(&feature));
            assert_eq!(get_mantissa_scale(), MantissaScale::Small);
        });

        assert_eq!(
            get_current_transaction_rules(),
            Some(Rules::new([sample_uint256(0xC0)]))
        );
        assert_eq!(get_mantissa_scale(), MantissaScale::Small);

        with_local_slot_owner(&owner, || {
            assert_eq!(get_current_transaction_rules(), Some(rules.clone()));
            assert!(is_feature_enabled(&feature));
            assert_eq!(get_mantissa_scale(), MantissaScale::Small);
        });

        set_current_transaction_rules(None);
    }
}
