//! Compact diagnostics for candidate transactions crossing captured-consensus
//! admission and inclusion.
//!
//! Enable with `XRPLD_CANDIDATE_DIAGNOSTICS=info` (or `1`/`true`) for info
//! records, or `XRPLD_CANDIDATE_DIAGNOSTICS=debug` for debug records. The
//! record key is the transaction ID, ledger sequence, source transaction
//! sequence, and captured transaction-set source.

use std::sync::OnceLock;

use basics::base_uint::Uint256;
use protocol::{Ter, trans_token};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateDiagnosticSource {
    CapturedConsensus,
}

impl CandidateDiagnosticSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CapturedConsensus => "captured_consensus",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateDiagnosticDecision {
    Accepted,
    Retry,
    Terminal,
    AlreadyInParent,
}

impl CandidateDiagnosticDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Retry => "retry",
            Self::Terminal => "terminal",
            Self::AlreadyInParent => "terminal_parent_exists",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateDiagnosticLogLevel {
    Debug,
    Info,
}

/// One deterministic candidate attempt. `None` means a stage did not run.
/// `accepted_index` is the zero-based outer transaction index used by
/// `TxMeta::add_raw`; Batch inner transactions remain metadata-only and are
/// intentionally not reported as independent outer candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandidateAdmissionDiagnostic {
    pub transaction_id: Uint256,
    pub ledger_sequence: u32,
    pub sequence: u32,
    pub source: CandidateDiagnosticSource,
    pub pass: usize,
    pub preflight: Option<Ter>,
    pub preclaim: Option<Ter>,
    pub apply: Option<Ter>,
    pub decision: CandidateDiagnosticDecision,
    pub accepted_index: Option<u32>,
}

impl CandidateAdmissionDiagnostic {
    pub const fn skipped_existing(
        transaction_id: Uint256,
        ledger_sequence: u32,
        sequence: u32,
        pass: usize,
    ) -> Self {
        Self {
            transaction_id,
            ledger_sequence,
            sequence,
            source: CandidateDiagnosticSource::CapturedConsensus,
            pass,
            preflight: None,
            preclaim: None,
            apply: None,
            decision: CandidateDiagnosticDecision::AlreadyInParent,
            accepted_index: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub const fn attempted(
        transaction_id: Uint256,
        ledger_sequence: u32,
        sequence: u32,
        pass: usize,
        preflight: Ter,
        preclaim: Option<Ter>,
        apply: Option<Ter>,
        decision: CandidateDiagnosticDecision,
        accepted_index: Option<u32>,
    ) -> Self {
        Self {
            transaction_id,
            ledger_sequence,
            sequence,
            source: CandidateDiagnosticSource::CapturedConsensus,
            pass,
            preflight: Some(preflight),
            preclaim,
            apply,
            decision,
            accepted_index,
        }
    }

    /// Stable compact text for line-oriented canonical-set comparison.
    pub fn compact(self) -> String {
        format!(
            "candidate tx={} ledger_seq={} sequence={} source={} pass={} preflight={} preclaim={} apply={} decision={} accepted_index={}",
            self.transaction_id,
            self.ledger_sequence,
            self.sequence,
            self.source.as_str(),
            self.pass,
            ter_token(self.preflight),
            ter_token(self.preclaim),
            ter_token(self.apply),
            self.decision.as_str(),
            self.accepted_index
                .map_or_else(|| "-".to_owned(), |index| index.to_string()),
        )
    }
}

fn ter_token(ter: Option<Ter>) -> &'static str {
    ter.map(trans_token).unwrap_or("-")
}

fn candidate_diagnostic_log_level_from_value(
    value: Option<&str>,
) -> Option<CandidateDiagnosticLogLevel> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.eq_ignore_ascii_case("debug") => {
            Some(CandidateDiagnosticLogLevel::Debug)
        }
        Some(value)
            if value.eq_ignore_ascii_case("info")
                || value == "1"
                || value.eq_ignore_ascii_case("true") =>
        {
            Some(CandidateDiagnosticLogLevel::Info)
        }
        _ => None,
    }
}

fn candidate_diagnostic_log_level() -> Option<CandidateDiagnosticLogLevel> {
    static LEVEL: OnceLock<Option<CandidateDiagnosticLogLevel>> = OnceLock::new();
    *LEVEL.get_or_init(|| {
        candidate_diagnostic_log_level_from_value(
            std::env::var("XRPLD_CANDIDATE_DIAGNOSTICS").ok().as_deref(),
        )
    })
}

pub fn emit_candidate_admission_diagnostic(diagnostic: CandidateAdmissionDiagnostic) {
    let message = diagnostic.compact();
    match candidate_diagnostic_log_level() {
        Some(CandidateDiagnosticLogLevel::Info) => tracing::info!(target: "candidate", "{message}"),
        Some(CandidateDiagnosticLogLevel::Debug) => {
            tracing::debug!(target: "candidate", "{message}")
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CandidateAdmissionDiagnostic, CandidateDiagnosticDecision,
        candidate_diagnostic_log_level_from_value,
    };
    use basics::base_uint::Uint256;
    use protocol::Ter;

    #[test]
    fn compact_attempt_records_all_candidate_comparison_fields() {
        let diagnostic = CandidateAdmissionDiagnostic::attempted(
            Uint256::from_u64(0xB1DF),
            106_121_261,
            42,
            2,
            Ter::TES_SUCCESS,
            Some(Ter::TES_SUCCESS),
            Some(Ter::TEC_PATH_DRY),
            CandidateDiagnosticDecision::Terminal,
            Some(7),
        );

        assert_eq!(
            diagnostic.compact(),
            "candidate tx=000000000000000000000000000000000000000000000000000000000000B1DF ledger_seq=106121261 sequence=42 source=captured_consensus pass=2 preflight=tesSUCCESS preclaim=tesSUCCESS apply=tecPATH_DRY decision=terminal accepted_index=7"
        );
    }

    #[test]
    fn retry_and_parent_duplicate_mark_unreached_stages_explicitly() {
        let retry = CandidateAdmissionDiagnostic::attempted(
            Uint256::from_u64(9),
            10,
            4,
            0,
            Ter::TES_SUCCESS,
            Some(Ter::TER_PRE_SEQ),
            None,
            CandidateDiagnosticDecision::Retry,
            None,
        );
        let existing =
            CandidateAdmissionDiagnostic::skipped_existing(Uint256::from_u64(10), 10, 5, 0);

        assert!(
            retry
                .compact()
                .contains("apply=- decision=retry accepted_index=-")
        );
        assert!(
            existing
                .compact()
                .contains("preflight=- preclaim=- apply=- decision=terminal_parent_exists")
        );
    }

    #[test]
    fn environment_value_selects_info_or_debug_without_enabling_by_default() {
        assert!(candidate_diagnostic_log_level_from_value(Some("debug")).is_some());
        assert!(candidate_diagnostic_log_level_from_value(Some("true")).is_some());
        assert_eq!(candidate_diagnostic_log_level_from_value(Some("0")), None);
        assert_eq!(candidate_diagnostic_log_level_from_value(None), None);
    }
}
