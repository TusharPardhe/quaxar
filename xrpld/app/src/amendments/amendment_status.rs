//! App-owned amendment table and warning state.
//!
//! This now carries the real owner responsibilities that the earlier
//! placeholder skipped:
//! - supported / enabled amendment ownership,
//! - trusted validation vote retention across rounds,
//! - amendment pseudo-transaction action calculation,
//! - validation amendment field generation,
//! - and unsupported-majority warning state for server-info style surfaces.

use crate::tx_queue::vote_tx_set::VoteTxSet;
use basics::base_uint::Uint256;
use basics::chrono::{NetClockTimePoint, TimeFormat, weeks};
use ledger::Ledger;
use protocol::{
    AccountID, ENABLE_AMENDMENT_GOT_MAJORITY_FLAG, ENABLE_AMENDMENT_LOST_MAJORITY_FLAG, JsonValue,
    PublicKey, RegisteredFeatureVote, STTx, STValidation, STVector256, TxType, feature_id,
    feature_name, get_field_by_symbol,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use time::Duration;

const TRUSTED_VOTE_EXPIRATION: Duration = Duration::hours(24);
const AMENDMENT_THRESHOLD_NUMERATOR: usize = 80;
const AMENDMENT_THRESHOLD_DENOMINATOR: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedMajorityWarningDetails {
    pub expected_date: i64,
    pub expected_date_utc: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct UnsupportedMajorityWarningState {
    warned: bool,
    details: Option<UnsupportedMajorityWarningDetails>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AmendmentVote {
    Obsolete,
    Up,
    #[default]
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownAmendment {
    pub name: String,
    pub amendment: Uint256,
    pub supported: bool,
    pub vote: AmendmentVote,
}

impl KnownAmendment {
    pub fn new(
        name: impl Into<String>,
        amendment: Uint256,
        supported: bool,
        vote: AmendmentVote,
    ) -> Self {
        Self {
            name: name.into(),
            amendment,
            supported,
            vote,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AmendmentState {
    vote: AmendmentVote,
    enabled: bool,
    supported: bool,
    name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TrustedValidatorVotes {
    up_votes: Vec<Uint256>,
    timeout: Option<NetClockTimePoint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmendmentLastVote {
    pub trusted_validations: usize,
    pub threshold: usize,
    pub votes: BTreeMap<Uint256, usize>,
}

#[derive(Debug, Default)]
struct AmendmentStatusState {
    amendments: BTreeMap<Uint256, AmendmentState>,
    trusted_validators: HashSet<PublicKey>,
    recorded_votes: HashMap<PublicKey, TrustedValidatorVotes>,
    last_update_seq: u32,
    last_vote: Option<AmendmentLastVote>,
    majority_amendments: BTreeMap<Uint256, NetClockTimePoint>,
    unsupported_enabled: bool,
    first_unsupported_expected: Option<NetClockTimePoint>,
    unsupported_majority_warning: UnsupportedMajorityWarningState,
}

#[derive(Debug)]
pub struct AmendmentStatus {
    majority_time: Duration,
    inner: Mutex<AmendmentStatusState>,
}

impl Default for AmendmentStatus {
    fn default() -> Self {
        Self::with_known_amendments(weeks(2), default_known_amendments())
    }
}

impl AmendmentStatus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_known_amendments<I>(majority_time: Duration, amendments: I) -> Self
    where
        I: IntoIterator<Item = KnownAmendment>,
    {
        let mut inner = AmendmentStatusState::default();
        for amendment in amendments {
            let state = inner.amendments.entry(amendment.amendment).or_default();
            state.name = amendment.name;
            state.supported = amendment.supported;
            state.vote = amendment.vote;
        }

        Self {
            majority_time,
            inner: Mutex::new(inner),
        }
    }

    pub fn majority_time(&self) -> Duration {
        self.majority_time
    }

    pub fn find(&self, name: &str) -> Option<Uint256> {
        let inner = self.inner.lock().expect("amendment status lock");
        inner
            .amendments
            .iter()
            .find_map(|(amendment, state)| (state.name == name).then_some(*amendment))
    }

    pub fn set_vote(&self, amendment: Uint256, vote: AmendmentVote) {
        let mut inner = self.inner.lock().expect("amendment status lock");
        let state = inner.amendments.entry(amendment).or_default();
        if state.name.is_empty() {
            state.name = feature_name(&amendment)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| amendment.to_string());
        }
        if state.vote != AmendmentVote::Obsolete {
            state.vote = vote;
        }
    }

    pub fn set_trusted_validators<I>(&self, validators: I)
    where
        I: IntoIterator<Item = PublicKey>,
    {
        let validators = validators.into_iter().collect::<HashSet<_>>();
        let mut inner = self.inner.lock().expect("amendment status lock");
        inner.trusted_validators = validators;
        let trusted_validators = inner.trusted_validators.clone();
        inner
            .recorded_votes
            .retain(|public_key, _| trusted_validators.contains(public_key));
    }

    pub fn enable(&self, amendment: Uint256) -> bool {
        let mut inner = self.inner.lock().expect("amendment status lock");
        enable_locked(&mut inner, amendment)
    }

    pub fn is_enabled(&self, amendment: Uint256) -> bool {
        self.inner
            .lock()
            .expect("amendment status lock")
            .amendments
            .get(&amendment)
            .is_some_and(|state| state.enabled)
    }

    pub fn is_supported(&self, amendment: Uint256) -> bool {
        self.inner
            .lock()
            .expect("amendment status lock")
            .amendments
            .get(&amendment)
            .is_some_and(|state| state.supported)
    }

    pub fn has_unsupported_enabled(&self) -> bool {
        self.inner
            .lock()
            .expect("amendment status lock")
            .unsupported_enabled
    }

    pub fn first_unsupported_expected(&self) -> Option<NetClockTimePoint> {
        self.inner
            .lock()
            .expect("amendment status lock")
            .first_unsupported_expected
    }

    pub fn unsupported_majority_warned(&self) -> bool {
        self.inner
            .lock()
            .expect("amendment status lock")
            .unsupported_majority_warning
            .warned
    }

    pub fn unsupported_majority_warning_details(
        &self,
    ) -> Option<UnsupportedMajorityWarningDetails> {
        self.inner
            .lock()
            .expect("amendment status lock")
            .unsupported_majority_warning
            .details
            .clone()
    }

    pub fn set_unsupported_majority_warning_details(
        &self,
        warning: Option<UnsupportedMajorityWarningDetails>,
    ) -> Option<UnsupportedMajorityWarningDetails> {
        let mut inner = self.inner.lock().expect("amendment status lock");
        if warning.is_some() {
            inner.unsupported_majority_warning.warned = true;
        }
        std::mem::replace(&mut inner.unsupported_majority_warning.details, warning)
    }

    pub fn set_unsupported_majority_warned(&self, warned: bool) -> bool {
        let mut inner = self.inner.lock().expect("amendment status lock");
        let previous = inner.unsupported_majority_warning.warned;
        inner.unsupported_majority_warning.warned = warned;
        if !warned {
            inner.unsupported_majority_warning.details = None;
        }
        previous
    }

    pub fn sync_warning_state_for_validated_ledger(&self, ledger: &Ledger) {
        if self.has_unsupported_enabled() {
            return;
        }

        if !self.unsupported_majority_warned() || ledger.is_flag_ledger() {
            if let Some(first) = self.first_unsupported_expected() {
                let _ = self.set_unsupported_majority_warning_details(Some(
                    UnsupportedMajorityWarningDetails {
                        expected_date: i64::from(first.as_seconds()),
                        expected_date_utc: first.to_xrpl_string(),
                    },
                ));
            } else {
                let _ = self.set_unsupported_majority_warned(false);
            }
        }
    }

    pub fn need_validated_ledger(&self, ledger_seq: u32) -> bool {
        let inner = self.inner.lock().expect("amendment status lock");
        ((ledger_seq.saturating_sub(1)) / 256) != ((inner.last_update_seq.saturating_sub(1)) / 256)
    }

    pub fn do_validated_ledger_with_sets(
        &self,
        ledger_seq: u32,
        enabled: &BTreeSet<Uint256>,
        majority: &BTreeMap<Uint256, NetClockTimePoint>,
    ) {
        let mut inner = self.inner.lock().expect("amendment status lock");
        for amendment in enabled {
            let _ = enable_locked(&mut inner, *amendment);
        }

        inner.last_update_seq = ledger_seq;
        inner.majority_amendments = majority.clone();
        inner.first_unsupported_expected = None;

        for (&amendment, &when) in majority {
            let state = inner.amendments.entry(amendment).or_default();
            if state.enabled {
                continue;
            }
            if !state.supported {
                inner.first_unsupported_expected = Some(
                    inner
                        .first_unsupported_expected
                        .map_or(when, |current| current.min(when)),
                );
            }
        }

        if let Some(first) = inner.first_unsupported_expected {
            inner.first_unsupported_expected = first.checked_add(self.majority_time);
        }
    }

    pub fn do_validated_ledger(&self, ledger: &Ledger) {
        if !self.need_validated_ledger(ledger.header().seq) {
            return;
        }

        let enabled = ledger.get_enabled_amendments();
        let majority = ledger.get_majority_amendments();
        self.do_validated_ledger_with_sets(ledger.header().seq, &enabled, &majority);
    }

    pub fn do_validation(&self, enabled: &BTreeSet<Uint256>) -> Vec<Uint256> {
        let inner = self.inner.lock().expect("amendment status lock");
        let mut amendments = Vec::new();
        for (&amendment, state) in &inner.amendments {
            if state.supported && state.vote == AmendmentVote::Up && !enabled.contains(&amendment) {
                amendments.push(amendment);
            }
        }
        amendments
    }

    pub fn do_validation_for_ledger(&self, ledger: &Ledger, validation: &mut STValidation) {
        let amendments = self.do_validation(&ledger.get_enabled_amendments());
        if !amendments.is_empty() {
            validation.set_field_v256(
                get_field_by_symbol("sfAmendments"),
                STVector256::from_values(get_field_by_symbol("sfAmendments"), amendments),
            );
        }
    }

    pub fn do_voting(
        &self,
        close_time: NetClockTimePoint,
        enabled_amendments: &BTreeSet<Uint256>,
        majority_amendments: &BTreeMap<Uint256, NetClockTimePoint>,
        validations: &[STValidation],
    ) -> BTreeMap<Uint256, u32> {
        let mut inner = self.inner.lock().expect("amendment status lock");
        record_votes_locked(&mut inner, close_time, validations);

        let last_vote = tally_votes(&inner);
        let mut actions = BTreeMap::new();

        for (&amendment, state) in &inner.amendments {
            if enabled_amendments.contains(&amendment) {
                continue;
            }

            let votes_for = last_vote.votes.get(&amendment).copied().unwrap_or_default();
            let has_validation_majority = passes_majority(
                last_vote.trusted_validations,
                last_vote.threshold,
                votes_for,
            );
            let has_ledger_majority = majority_amendments.contains_key(&amendment);

            if has_validation_majority && !has_ledger_majority && state.vote == AmendmentVote::Up {
                tracing::info!(target: "consensus", name = %amendment, "Amendment majority reached");
                actions.insert(amendment, ENABLE_AMENDMENT_GOT_MAJORITY_FLAG);
            } else if !has_validation_majority && has_ledger_majority {
                actions.insert(amendment, ENABLE_AMENDMENT_LOST_MAJORITY_FLAG);
            } else if has_ledger_majority
                && state.vote == AmendmentVote::Up
                && majority_amendments
                    .get(&amendment)
                    .and_then(|majority_time| majority_time.checked_add(self.majority_time))
                    .is_some_and(|majority_holds_until| majority_holds_until <= close_time)
            {
                tracing::info!(target: "consensus", name = %amendment, "Amendment activated");
                actions.insert(amendment, 0);
            }
        }

        inner.last_vote = Some(last_vote);
        actions
    }

    pub fn do_voting_for_ledger<S>(
        &self,
        last_closed_ledger: &Ledger,
        parent_validations: &[STValidation],
        initial_position: &mut S,
    ) -> BTreeMap<Uint256, u32>
    where
        S: VoteTxSet,
    {
        assert!(
            last_closed_ledger.is_flag_ledger(),
            "xrpl::AmendmentTable::doVoting : has a flag ledger"
        );

        let actions = self.do_voting(
            NetClockTimePoint::new(last_closed_ledger.header().parent_close_time),
            &last_closed_ledger.get_enabled_amendments(),
            &last_closed_ledger.get_majority_amendments(),
            parent_validations,
        );

        for (&amendment, &flags) in &actions {
            let tx = STTx::new(TxType::AMENDMENT, |tx| {
                tx.set_account_id(get_field_by_symbol("sfAccount"), AccountID::default());
                tx.set_field_h256(get_field_by_symbol("sfAmendment"), amendment);
                tx.set_field_u32(
                    get_field_by_symbol("sfLedgerSequence"),
                    last_closed_ledger.header().seq + 1,
                );
                if flags != 0 {
                    tx.set_field_u32(get_field_by_symbol("sfFlags"), flags);
                }
            });
            let _ = initial_position.add_transaction(&tx);
        }

        actions
    }

    pub fn get_desired(&self) -> Vec<Uint256> {
        self.do_validation(&BTreeSet::new())
    }

    pub fn last_vote(&self) -> Option<AmendmentLastVote> {
        self.inner
            .lock()
            .expect("amendment status lock")
            .last_vote
            .clone()
    }

    pub fn majority_timestamps(&self) -> BTreeMap<Uint256, i64> {
        self.inner
            .lock()
            .expect("amendment status lock")
            .majority_amendments
            .iter()
            .map(|(feature, when)| (*feature, i64::from(when.as_seconds())))
            .collect()
    }

    pub fn feature_table_json(&self, is_admin: bool) -> JsonValue {
        let inner = self.inner.lock().expect("amendment status lock");
        let mut features = BTreeMap::new();
        for (&amendment, state) in &inner.amendments {
            features.insert(
                amendment.to_string(),
                feature_state_json(
                    state,
                    &inner.last_vote,
                    amendment,
                    is_admin,
                    inner.majority_amendments.get(&amendment).copied(),
                ),
            );
        }
        JsonValue::Object(features)
    }

    pub fn feature_json(&self, amendment: Uint256, is_admin: bool) -> Option<JsonValue> {
        let inner = self.inner.lock().expect("amendment status lock");
        let state = inner.amendments.get(&amendment)?;
        let majority = inner.majority_amendments.get(&amendment).copied();
        let mut reply = BTreeMap::new();
        reply.insert(
            amendment.to_string(),
            feature_state_json(state, &inner.last_vote, amendment, is_admin, majority),
        );
        Some(JsonValue::Object(reply))
    }
}

fn feature_state_json(
    state: &AmendmentState,
    last_vote: &Option<AmendmentLastVote>,
    amendment: Uint256,
    is_admin: bool,
    majority: Option<NetClockTimePoint>,
) -> JsonValue {
    let mut object = BTreeMap::new();
    if !state.name.is_empty() {
        object.insert("name".to_owned(), JsonValue::String(state.name.clone()));
    }
    object.insert("supported".to_owned(), JsonValue::Bool(state.supported));
    if !state.enabled && is_admin {
        object.insert(
            "vetoed".to_owned(),
            match state.vote {
                AmendmentVote::Obsolete => JsonValue::String("Obsolete".to_owned()),
                AmendmentVote::Down => JsonValue::Bool(true),
                AmendmentVote::Up => JsonValue::Bool(false),
            },
        );
    }
    object.insert("enabled".to_owned(), JsonValue::Bool(state.enabled));
    if !state.enabled
        && is_admin
        && let Some(last_vote) = last_vote
    {
        object.insert(
            "count".to_owned(),
            JsonValue::Unsigned(
                last_vote
                    .votes
                    .get(&amendment)
                    .copied()
                    .unwrap_or_default() as u64,
            ),
        );
        object.insert(
            "validations".to_owned(),
            JsonValue::Unsigned(last_vote.trusted_validations as u64),
        );
        if last_vote.threshold != 0 {
            object.insert(
                "threshold".to_owned(),
                JsonValue::Unsigned(last_vote.threshold as u64),
            );
        }
    }
    if let Some(majority_time) = majority {
        object.insert(
            "majority".to_owned(),
            JsonValue::Signed(majority_time.as_seconds() as i64),
        );
    }
    JsonValue::Object(object)
}

fn enable_locked(inner: &mut AmendmentStatusState, amendment: Uint256) -> bool {
    let state = inner.amendments.entry(amendment).or_default();
    if state.enabled {
        return false;
    }

    if state.name.is_empty() {
        state.name = feature_name(&amendment)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| amendment.to_string());
    }
    state.enabled = true;
    if !state.supported {
        inner.unsupported_enabled = true;
    }
    true
}

fn record_votes_locked(
    inner: &mut AmendmentStatusState,
    close_time: NetClockTimePoint,
    validations: &[STValidation],
) {
    if !inner.trusted_validators.is_empty() {
        let trusted_validators = inner.trusted_validators.clone();
        inner
            .recorded_votes
            .retain(|public_key, _| trusted_validators.contains(public_key));
    }

    let new_timeout = close_time + TRUSTED_VOTE_EXPIRATION;
    for validation in validations
        .iter()
        .filter(|validation| validation.is_trusted())
    {
        let signer = *validation.get_signer_public();
        if !inner.trusted_validators.is_empty() && !inner.trusted_validators.contains(&signer) {
            continue;
        }

        let votes = if validation.is_field_present(get_field_by_symbol("sfAmendments")) {
            validation
                .get_field_v256(get_field_by_symbol("sfAmendments"))
                .value()
                .to_vec()
        } else {
            Vec::new()
        };

        let entry = inner.recorded_votes.entry(signer).or_default();
        entry.timeout = Some(new_timeout);
        entry.up_votes = votes;
    }

    for votes in inner.recorded_votes.values_mut() {
        match votes.timeout {
            None => {
                votes.up_votes.clear();
            }
            Some(timeout) if close_time > timeout => {
                votes.timeout = None;
                votes.up_votes.clear();
            }
            Some(_) => {}
        }
    }
}

fn tally_votes(inner: &AmendmentStatusState) -> AmendmentLastVote {
    let mut votes = BTreeMap::new();
    let mut trusted_validations = 0usize;

    for validator_votes in inner.recorded_votes.values() {
        if validator_votes.timeout.is_some() {
            trusted_validations += 1;
            for amendment in &validator_votes.up_votes {
                *votes.entry(*amendment).or_default() += 1;
            }
        }
    }

    let threshold = usize::max(
        1,
        (trusted_validations * AMENDMENT_THRESHOLD_NUMERATOR) / AMENDMENT_THRESHOLD_DENOMINATOR,
    );

    AmendmentLastVote {
        trusted_validations,
        threshold,
        votes,
    }
}

fn passes_majority(trusted_validations: usize, threshold: usize, votes_for: usize) -> bool {
    if trusted_validations == 1 {
        votes_for >= threshold
    } else {
        votes_for > threshold
    }
}

fn default_known_amendments() -> Vec<KnownAmendment> {
    protocol::REGISTERED_FEATURES
        .iter()
        .map(|registered| {
            KnownAmendment::new(
                registered.name,
                feature_id(registered.name),
                registered.supported,
                match registered.vote {
                    RegisteredFeatureVote::DefaultYes => AmendmentVote::Up,
                    RegisteredFeatureVote::DefaultNo => AmendmentVote::Down,
                    RegisteredFeatureVote::Obsolete => AmendmentVote::Obsolete,
                },
            )
        })
        .collect()
}
