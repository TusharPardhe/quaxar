//! RPC tuning constants ported from `xrpld/rpc/detail/Tuning.h`.

#![allow(dead_code)]

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitRange {
    pub rmin: u32,
    pub r_default: u32,
    pub rmax: u32,
}

pub struct Tuning;

impl Tuning {
    pub const ACCOUNT_LINES: LimitRange = LimitRange {
        rmin: 10,
        r_default: 200,
        rmax: 400,
    };
    pub const ACCOUNT_CHANNELS: LimitRange = LimitRange {
        rmin: 10,
        r_default: 200,
        rmax: 400,
    };
    pub const ACCOUNT_OBJECTS: LimitRange = LimitRange {
        rmin: 10,
        r_default: 200,
        rmax: 400,
    };
    pub const ACCOUNT_OFFERS: LimitRange = LimitRange {
        rmin: 10,
        r_default: 200,
        rmax: 400,
    };
    pub const ACCOUNT_TX: LimitRange = LimitRange {
        rmin: 10,
        r_default: 200,
        rmax: 400,
    };
    pub const BOOK_OFFERS: LimitRange = LimitRange {
        rmin: 0,
        r_default: 60,
        rmax: 100,
    };
    pub const NO_RIPPLE_CHECK: LimitRange = LimitRange {
        rmin: 10,
        r_default: 300,
        rmax: 400,
    };
    pub const ACCOUNT_NFTOKENS: LimitRange = LimitRange {
        rmin: 20,
        r_default: 100,
        rmax: 400,
    };
    pub const NFT_OFFERS: LimitRange = LimitRange {
        rmin: 50,
        r_default: 250,
        rmax: 500,
    };

    pub const DEFAULT_AUTO_FILL_FEE_MULTIPLIER: u32 = 10;
    pub const DEFAULT_AUTO_FILL_FEE_DIVISOR: u32 = 1;
    pub const MAX_PATHFINDS_IN_PROGRESS: u32 = 2;
    pub const MAX_PATHFIND_JOB_COUNT: u32 = 50;
    pub const MAX_JOB_QUEUE_CLIENTS: u32 = 500;
    pub const MAX_VALIDATED_LEDGER_AGE: Duration = Duration::from_secs(120);
    pub const MAX_REQUEST_SIZE: u32 = 1_000_000;
    pub const BINARY_PAGE_LENGTH: u32 = 2048;
    pub const JSON_PAGE_LENGTH: u32 = 256;
    pub const MAX_SRC_CUR: u32 = 18;
    pub const MAX_AUTO_SRC_CUR: u32 = 88;

    pub const fn page_length(is_binary: bool) -> u32 {
        if is_binary {
            Self::BINARY_PAGE_LENGTH
        } else {
            Self::JSON_PAGE_LENGTH
        }
    }
}

pub const fn binary_page_length() -> u32 {
    Tuning::BINARY_PAGE_LENGTH
}

pub const fn json_page_length() -> u32 {
    Tuning::JSON_PAGE_LENGTH
}

pub const fn page_length(is_binary: bool) -> u32 {
    Tuning::page_length(is_binary)
}

#[cfg(test)]
mod tests {
    use super::{LimitRange, Tuning, binary_page_length, json_page_length};
    use std::time::Duration;

    #[test]
    fn constants_match_cpp_values() {
        assert_eq!(
            Tuning::ACCOUNT_LINES,
            LimitRange {
                rmin: 10,
                r_default: 200,
                rmax: 400,
            }
        );
        assert_eq!(Tuning::BOOK_OFFERS.r_default, 60);
        assert_eq!(Tuning::ACCOUNT_NFTOKENS.rmin, 20);
        assert_eq!(Tuning::NFT_OFFERS.rmax, 500);
        assert_eq!(Tuning::DEFAULT_AUTO_FILL_FEE_MULTIPLIER, 10);
        assert_eq!(Tuning::DEFAULT_AUTO_FILL_FEE_DIVISOR, 1);
        assert_eq!(Tuning::MAX_PATHFINDS_IN_PROGRESS, 2);
        assert_eq!(Tuning::MAX_PATHFIND_JOB_COUNT, 50);
        assert_eq!(Tuning::MAX_JOB_QUEUE_CLIENTS, 500);
        assert_eq!(Tuning::MAX_VALIDATED_LEDGER_AGE, Duration::from_secs(120));
        assert_eq!(Tuning::MAX_REQUEST_SIZE, 1_000_000);
        assert_eq!(Tuning::MAX_SRC_CUR, 18);
        assert_eq!(Tuning::MAX_AUTO_SRC_CUR, 88);
        assert_eq!(binary_page_length(), 2048);
        assert_eq!(json_page_length(), 256);
        assert_eq!(Tuning::page_length(true), 2048);
        assert_eq!(Tuning::page_length(false), 256);
    }
}
