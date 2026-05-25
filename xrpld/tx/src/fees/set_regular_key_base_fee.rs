//! Current the reference implementation wrapper.
//!
//! This ports the deterministic outer behavior around:
//!
//! - charging zero fee only when the signing public key resolves to the
//!   submitting account,
//! - requiring the account root to exist for that zero-fee path, and
//! - requiring `lsfPasswordSpent` to still be clear before returning zero.

pub trait SetRegularKeyBaseFeeAccountState {
    fn password_spent(&self) -> bool;
}

pub fn run_set_regular_key_calculate_base_fee<AccountState, Fee>(
    transactor_base_fee: Fee,
    zero_fee: Fee,
    signing_pub_key_matches_tx_account: bool,
    account_state: Option<AccountState>,
) -> Fee
where
    Fee: Copy,
    AccountState: SetRegularKeyBaseFeeAccountState,
{
    if signing_pub_key_matches_tx_account
        && let Some(account_state) = account_state
        && !account_state.password_spent()
    {
        return zero_fee;
    }

    transactor_base_fee
}

#[cfg(test)]
mod tests {
    use super::{SetRegularKeyBaseFeeAccountState, run_set_regular_key_calculate_base_fee};

    #[derive(Clone, Copy)]
    struct TestAccountState {
        password_spent: bool,
    }

    impl SetRegularKeyBaseFeeAccountState for TestAccountState {
        fn password_spent(&self) -> bool {
            self.password_spent
        }
    }

    #[test]
    fn set_regular_key_calculate_base_fee_is_zero_for_matching_master_key_before_password_spend() {
        let fee = run_set_regular_key_calculate_base_fee(
            10_u64,
            0_u64,
            true,
            Some(TestAccountState {
                password_spent: false,
            }),
        );

        assert_eq!(fee, 0);
    }

    #[test]
    fn set_regular_key_calculate_base_fee_is_normal_when_signing_key_does_not_match() {
        let fee = run_set_regular_key_calculate_base_fee(
            10_u64,
            0_u64,
            false,
            Some(TestAccountState {
                password_spent: false,
            }),
        );

        assert_eq!(fee, 10);
    }

    #[test]
    fn set_regular_key_calculate_base_fee_is_normal_when_account_lookup_misses() {
        let fee =
            run_set_regular_key_calculate_base_fee(10_u64, 0_u64, true, None::<TestAccountState>);

        assert_eq!(fee, 10);
    }

    #[test]
    fn set_regular_key_calculate_base_fee_is_normal_after_password_spend() {
        let fee = run_set_regular_key_calculate_base_fee(
            10_u64,
            0_u64,
            true,
            Some(TestAccountState {
                password_spent: true,
            }),
        );

        assert_eq!(fee, 10);
    }
}
