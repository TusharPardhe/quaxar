//! Current Rust helper mirroring `Transactor::checkSingleSign(...)`.
//!
//! This module preserves the exact current authorization order around:
//!
//! - accepting the ledger regular key,
//! - accepting an enabled master key,
//! - rejecting a disabled master key with `tefMASTER_DISABLED`, and
//! - rejecting every other signer with `tefBAD_AUTH`.

use protocol::{NotTec, Ter};

pub trait TransactorSingleSignAccountState<AccountId> {
    fn regular_key(&self) -> Option<&AccountId>;
    fn is_master_disabled(&self) -> bool;
}

impl<AccountId> TransactorSingleSignAccountState<AccountId> for () {
    fn regular_key(&self) -> Option<&AccountId> {
        None
    }

    fn is_master_disabled(&self) -> bool {
        false
    }
}

pub fn run_transactor_check_single_sign<AccountId, AccountState>(
    id_signer: &AccountId,
    id_account: &AccountId,
    account_state: &AccountState,
) -> NotTec
where
    AccountId: Eq,
    AccountState: TransactorSingleSignAccountState<AccountId>,
{
    if account_state.regular_key() == Some(id_signer) {
        return Ter::TES_SUCCESS;
    }

    if !account_state.is_master_disabled() && id_account == id_signer {
        return Ter::TES_SUCCESS;
    }

    if account_state.is_master_disabled() && id_account == id_signer {
        return Ter::TEF_MASTER_DISABLED;
    }

    Ter::TEF_BAD_AUTH
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::{TransactorSingleSignAccountState, run_transactor_check_single_sign};

    struct TestAccountState {
        regular_key: Option<&'static str>,
        is_master_disabled: bool,
    }

    impl TransactorSingleSignAccountState<&'static str> for TestAccountState {
        fn regular_key(&self) -> Option<&&'static str> {
            self.regular_key.as_ref()
        }

        fn is_master_disabled(&self) -> bool {
            self.is_master_disabled
        }
    }

    #[test]
    fn transactor_single_sign_accepts_regular_key_before_master_checks() {
        let result = run_transactor_check_single_sign(
            &"regular",
            &"alice",
            &TestAccountState {
                regular_key: Some("regular"),
                is_master_disabled: true,
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(trans_token(result), "tesSUCCESS");
    }

    #[test]
    fn transactor_single_sign_accepts_enabled_master_key() {
        let result = run_transactor_check_single_sign(
            &"alice",
            &"alice",
            &TestAccountState {
                regular_key: Some("regular"),
                is_master_disabled: false,
            },
        );

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn transactor_single_sign_rejects_disabled_master_key() {
        let result = run_transactor_check_single_sign(
            &"alice",
            &"alice",
            &TestAccountState {
                regular_key: Some("regular"),
                is_master_disabled: true,
            },
        );

        assert_eq!(result, protocol::Ter::TEF_MASTER_DISABLED);
        assert_eq!(trans_token(result), "tefMASTER_DISABLED");
    }

    #[test]
    fn transactor_single_sign_rejects_unauthorized_signers() {
        let result = run_transactor_check_single_sign(
            &"mallory",
            &"alice",
            &TestAccountState {
                regular_key: Some("regular"),
                is_master_disabled: false,
            },
        );

        assert_eq!(result, protocol::Ter::TEF_BAD_AUTH);
        assert_eq!(trans_token(result), "tefBAD_AUTH");
    }
}
