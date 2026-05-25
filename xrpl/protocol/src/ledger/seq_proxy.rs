//! `xrpl/protocol/SeqProxy.h` compatibility surface.

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SeqProxyKind {
    Sequence = 0,
    Ticket = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeqProxy {
    value: u32,
    kind: SeqProxyKind,
}

impl SeqProxy {
    pub const fn new(kind: SeqProxyKind, value: u32) -> Self {
        Self { value, kind }
    }

    pub const fn sequence(value: u32) -> Self {
        Self::new(SeqProxyKind::Sequence, value)
    }

    pub const fn ticket(value: u32) -> Self {
        Self::new(SeqProxyKind::Ticket, value)
    }

    pub const fn value(self) -> u32 {
        self.value
    }

    pub const fn is_seq(self) -> bool {
        matches!(self.kind, SeqProxyKind::Sequence)
    }

    pub const fn is_ticket(self) -> bool {
        matches!(self.kind, SeqProxyKind::Ticket)
    }

    pub fn advance_by(&mut self, amount: u32) -> &mut Self {
        self.value = self.value.wrapping_add(amount);
        self
    }
}

impl PartialOrd for SeqProxy {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SeqProxy {
    fn cmp(&self, other: &Self) -> Ordering {
        self.kind
            .cmp(&other.kind)
            .then_with(|| self.value.cmp(&other.value))
    }
}

impl fmt::Display for SeqProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_seq() {
            write!(f, "sequence {}", self.value)
        } else {
            write!(f, "ticket {}", self.value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SeqProxy, SeqProxyKind};

    #[test]
    fn constructors_and_accessors_match_current_cpp_roles() {
        let sequence = SeqProxy::sequence(7);
        let ticket = SeqProxy::ticket(9);
        let direct = SeqProxy::new(SeqProxyKind::Ticket, 11);

        assert_eq!(sequence.value(), 7);
        assert!(sequence.is_seq());
        assert!(!sequence.is_ticket());

        assert_eq!(ticket.value(), 9);
        assert!(!ticket.is_seq());
        assert!(ticket.is_ticket());

        assert_eq!(direct, SeqProxy::ticket(11));
    }

    #[test]
    fn ordering_keeps_all_sequences_before_tickets() {
        assert!(SeqProxy::sequence(100) < SeqProxy::ticket(1));
        assert!(SeqProxy::sequence(1) < SeqProxy::sequence(2));
        assert!(SeqProxy::ticket(1) < SeqProxy::ticket(2));
    }

    #[test]
    fn advance_by_wraps_unsigned_arithmetic() {
        let mut seq = SeqProxy::sequence(u32::MAX);
        seq.advance_by(1);

        assert_eq!(seq, SeqProxy::sequence(0));
    }

    #[test]
    fn display_matches_current_cpp_stream_shape() {
        assert_eq!(SeqProxy::sequence(3).to_string(), "sequence 3");
        assert_eq!(SeqProxy::ticket(4).to_string(), "ticket 4");
    }
}
