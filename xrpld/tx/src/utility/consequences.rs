//! Deterministic `TxConsequences` carrier behavior from `xrpl/tx/applySteps.h`.
//!
//! This keeps the current consequence shape and sequencing rules while
//! intentionally storing fee and spend as raw XRP-drop counts until a real
//! `XRPAmount` port exists.

use protocol::{NotTec, SeqProxy, is_tes_success};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxConsequencesCategory {
    Normal,
    Blocker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxConsequencesShape {
    Normal,
    Blocker,
    PotentialSpend(u64),
    SequencesConsumed(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxConsequences {
    is_blocker: bool,
    fee_drops: u64,
    potential_spend_drops: u64,
    seq_proxy: SeqProxy,
    sequences_consumed: u32,
}

impl TxConsequences {
    pub fn from_preflight_result(preflight_result: NotTec) -> Self {
        assert!(
            !is_tes_success(preflight_result),
            "xrpl::TxConsequences::TxConsequences : is not tesSUCCESS"
        );

        Self {
            is_blocker: false,
            fee_drops: 0,
            potential_spend_drops: 0,
            seq_proxy: SeqProxy::sequence(0),
            sequences_consumed: 0,
        }
    }

    pub const fn new(fee_drops: u64, seq_proxy: SeqProxy) -> Self {
        Self {
            is_blocker: false,
            fee_drops,
            potential_spend_drops: 0,
            seq_proxy,
            sequences_consumed: if seq_proxy.is_seq() { 1 } else { 0 },
        }
    }

    pub const fn with_category(
        fee_drops: u64,
        seq_proxy: SeqProxy,
        category: TxConsequencesCategory,
    ) -> Self {
        let mut base = Self::new(fee_drops, seq_proxy);
        base.is_blocker = matches!(category, TxConsequencesCategory::Blocker);
        base
    }

    pub const fn with_potential_spend(
        fee_drops: u64,
        seq_proxy: SeqProxy,
        potential_spend_drops: u64,
    ) -> Self {
        let mut base = Self::new(fee_drops, seq_proxy);
        base.potential_spend_drops = potential_spend_drops;
        base
    }

    pub const fn with_sequences_consumed(
        fee_drops: u64,
        seq_proxy: SeqProxy,
        sequences_consumed: u32,
    ) -> Self {
        let mut base = Self::new(fee_drops, seq_proxy);
        base.sequences_consumed = sequences_consumed;
        base
    }

    pub const fn from_shape(
        fee_drops: u64,
        seq_proxy: SeqProxy,
        shape: TxConsequencesShape,
    ) -> Self {
        match shape {
            TxConsequencesShape::Normal => Self::new(fee_drops, seq_proxy),
            TxConsequencesShape::Blocker => {
                Self::with_category(fee_drops, seq_proxy, TxConsequencesCategory::Blocker)
            }
            TxConsequencesShape::PotentialSpend(potential_spend_drops) => {
                Self::with_potential_spend(fee_drops, seq_proxy, potential_spend_drops)
            }
            TxConsequencesShape::SequencesConsumed(sequences_consumed) => {
                Self::with_sequences_consumed(fee_drops, seq_proxy, sequences_consumed)
            }
        }
    }

    pub const fn fee(self) -> u64 {
        self.fee_drops
    }

    pub const fn potential_spend(self) -> u64 {
        self.potential_spend_drops
    }

    pub const fn seq_proxy(self) -> SeqProxy {
        self.seq_proxy
    }

    pub const fn sequences_consumed(self) -> u32 {
        self.sequences_consumed
    }

    pub const fn is_blocker(self) -> bool {
        self.is_blocker
    }

    pub fn following_seq(self) -> SeqProxy {
        let mut following = self.seq_proxy;
        following.advance_by(self.sequences_consumed);
        following
    }
}

pub const fn build_tx_consequences(
    fee_drops: u64,
    seq_proxy: SeqProxy,
    shape: TxConsequencesShape,
) -> TxConsequences {
    TxConsequences::from_shape(fee_drops, seq_proxy, shape)
}

#[cfg(test)]
mod tests {
    use super::{
        TxConsequences, TxConsequencesCategory, TxConsequencesShape, build_tx_consequences,
    };
    use protocol::{SeqProxy, Ter};

    #[test]
    fn preflight_failure_constructor_zeroes_fields() {
        let consequences = TxConsequences::from_preflight_result(Ter::TER_RETRY);

        assert!(!consequences.is_blocker());
        assert_eq!(consequences.fee(), 0);
        assert_eq!(consequences.potential_spend(), 0);
        assert_eq!(consequences.seq_proxy(), SeqProxy::sequence(0));
        assert_eq!(consequences.sequences_consumed(), 0);
    }

    #[test]
    #[should_panic(expected = "is not tesSUCCESS")]
    fn preflight_failure_constructor_rejects_success() {
        let _ = TxConsequences::from_preflight_result(Ter::TES_SUCCESS);
    }

    #[test]
    fn normal_constructor_defaults_match_current_cpp_roles() {
        let sequence = TxConsequences::new(12, SeqProxy::sequence(5));
        let ticket = TxConsequences::new(12, SeqProxy::ticket(5));

        assert_eq!(sequence.sequences_consumed(), 1);
        assert_eq!(sequence.following_seq(), SeqProxy::sequence(6));
        assert_eq!(ticket.sequences_consumed(), 0);
        assert_eq!(ticket.following_seq(), SeqProxy::ticket(5));
    }

    #[test]
    fn category_and_custom_overrides_match_current_cpp_roles() {
        let blocker = TxConsequences::with_category(
            9,
            SeqProxy::sequence(3),
            TxConsequencesCategory::Blocker,
        );
        let spending = TxConsequences::with_potential_spend(9, SeqProxy::sequence(3), 44);
        let multi_seq = TxConsequences::with_sequences_consumed(9, SeqProxy::sequence(3), 4);

        assert!(blocker.is_blocker());
        assert_eq!(spending.potential_spend(), 44);
        assert_eq!(multi_seq.following_seq(), SeqProxy::sequence(7));
    }

    #[test]
    fn build_tx_consequences_selects_current_shapes_helpers() {
        let normal = build_tx_consequences(1, SeqProxy::sequence(2), TxConsequencesShape::Normal);
        let blocker = build_tx_consequences(2, SeqProxy::sequence(3), TxConsequencesShape::Blocker);
        let spending = build_tx_consequences(
            3,
            SeqProxy::sequence(4),
            TxConsequencesShape::PotentialSpend(55),
        );
        let consumed = build_tx_consequences(
            4,
            SeqProxy::ticket(5),
            TxConsequencesShape::SequencesConsumed(6),
        );

        assert_eq!(normal, TxConsequences::new(1, SeqProxy::sequence(2)));
        assert_eq!(
            blocker,
            TxConsequences::with_category(
                2,
                SeqProxy::sequence(3),
                TxConsequencesCategory::Blocker
            )
        );
        assert_eq!(spending.potential_spend(), 55);
        assert_eq!(consumed.sequences_consumed(), 6);
        assert_eq!(consumed.following_seq(), SeqProxy::ticket(11));
    }
}
