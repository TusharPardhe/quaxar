//! `ApplyFlags` port from `xrpl/ledger/ApplyView.h`.

use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ApplyFlags(u32);

impl ApplyFlags {
    pub const NONE: Self = Self(0x00);
    pub const FAIL_HARD: Self = Self(0x10);
    pub const RETRY: Self = Self(0x20);
    pub const UNLIMITED: Self = Self(0x400);
    pub const BATCH: Self = Self(0x800);
    pub const DRY_RUN: Self = Self(0x1000);

    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl std::ops::BitOr for ApplyFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for ApplyFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = *self | rhs;
    }
}

impl std::ops::BitAnd for ApplyFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl std::ops::BitAndAssign for ApplyFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        *self = *self & rhs;
    }
}

impl std::ops::Not for ApplyFlags {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}

impl Display for ApplyFlags {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.bits())
    }
}

pub const fn any_apply_flags(flags: ApplyFlags) -> bool {
    flags.bits() != 0
}
