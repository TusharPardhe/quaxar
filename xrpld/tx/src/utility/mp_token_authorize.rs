//! Deterministic the reference implementation shells.
//!
//! This ports the current compatibility-safe surface for:
//!
//! - `getFlagsMask(...)`,
//! - `preflight(...)`,
//! - the ordered `preclaim(...)` holder and issuer branches,
//! - `createMPToken(...)`,
//! - and the thin `doApply()` wrapper.

use protocol::{NotTec, Ter, tfMPTUnauthorize, tfMPTUnauthorizeMask};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MPTokenAuthorizePreflightFacts {
    pub account_equals_holder: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MPTokenAuthorizePreclaimFacts {
    pub holder_present: bool,
    pub account_token_exists: bool,
    pub tx_flags: u32,
    pub token_balance_is_zero: bool,
    pub token_locked_amount_is_zero: bool,
    pub issuance_exists: bool,
    pub single_asset_vault_enabled: bool,
    pub token_locked: bool,
    pub account_is_issuer: bool,
    pub holder_account_exists: bool,
    pub issuance_requires_auth: bool,
    pub holder_token_exists: bool,
    pub holder_is_pseudo_account: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MPTokenAuthorizeCreateMutation<AccountId, MptId> {
    pub account: AccountId,
    pub mpt_issuance_id: MptId,
    pub flags: u32,
    pub owner_node: u64,
}

pub trait MPTokenAuthorizeCreateSink<AccountId, MptId> {
    fn insert_owner_dir(&mut self) -> Option<u64>;
    fn insert_mptoken(&mut self, mutation: MPTokenAuthorizeCreateMutation<AccountId, MptId>);
}

pub trait MPTokenAuthorizeApplySink<AccountId, MptId> {
    fn authorize_mptoken(
        &mut self,
        mpt_issuance_id: MptId,
        account: AccountId,
        tx_flags: u32,
        holder: Option<AccountId>,
    ) -> Ter;
}

pub const fn get_mp_token_authorize_flags_mask() -> u32 {
    tfMPTUnauthorizeMask
}

pub fn run_mp_token_authorize_preflight(facts: MPTokenAuthorizePreflightFacts) -> NotTec {
    if facts.account_equals_holder {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub fn run_mp_token_authorize_preclaim(facts: MPTokenAuthorizePreclaimFacts) -> Ter {
    if !facts.holder_present {
        if (facts.tx_flags & tfMPTUnauthorize) != 0 {
            if !facts.account_token_exists {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }

            if !facts.token_balance_is_zero {
                return if facts.issuance_exists {
                    Ter::TEC_HAS_OBLIGATIONS
                } else {
                    Ter::TEF_INTERNAL
                };
            }

            if !facts.token_locked_amount_is_zero {
                return if facts.issuance_exists {
                    Ter::TEC_HAS_OBLIGATIONS
                } else {
                    Ter::TEF_INTERNAL
                };
            }

            if facts.single_asset_vault_enabled && facts.token_locked {
                return Ter::TEC_NO_PERMISSION;
            }

            return Ter::TES_SUCCESS;
        }

        if !facts.issuance_exists {
            return Ter::TEC_OBJECT_NOT_FOUND;
        }

        if facts.account_is_issuer {
            return Ter::TEC_NO_PERMISSION;
        }

        if facts.account_token_exists {
            return Ter::TEC_DUPLICATE;
        }

        return Ter::TES_SUCCESS;
    }

    if !facts.holder_account_exists {
        return Ter::TEC_NO_DST;
    }

    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.account_is_issuer {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.issuance_requires_auth {
        return Ter::TEC_NO_AUTH;
    }

    if !facts.holder_token_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if facts.holder_is_pseudo_account {
        return Ter::TEC_NO_PERMISSION;
    }

    Ter::TES_SUCCESS
}

pub fn run_mp_token_authorize_create_mptoken<AccountId, MptId, S>(
    mpt_issuance_id: MptId,
    account: AccountId,
    flags: u32,
    sink: &mut S,
) -> Ter
where
    AccountId: Clone,
    MptId: Clone,
    S: MPTokenAuthorizeCreateSink<AccountId, MptId>,
{
    let Some(owner_node) = sink.insert_owner_dir() else {
        return Ter::TEC_DIR_FULL;
    };

    sink.insert_mptoken(MPTokenAuthorizeCreateMutation {
        account,
        mpt_issuance_id,
        flags,
        owner_node,
    });
    Ter::TES_SUCCESS
}

pub fn run_mp_token_authorize_do_apply<AccountId, MptId, S>(
    mpt_issuance_id: MptId,
    account: AccountId,
    tx_flags: u32,
    holder: Option<AccountId>,
    sink: &mut S,
) -> Ter
where
    AccountId: Clone,
    MptId: Clone,
    S: MPTokenAuthorizeApplySink<AccountId, MptId>,
{
    sink.authorize_mptoken(mpt_issuance_id, account, tx_flags, holder)
}

#[cfg(test)]
mod tests {
    use super::{
        MPTokenAuthorizeApplySink, MPTokenAuthorizeCreateMutation, MPTokenAuthorizeCreateSink,
        MPTokenAuthorizePreclaimFacts, MPTokenAuthorizePreflightFacts,
        get_mp_token_authorize_flags_mask, run_mp_token_authorize_create_mptoken,
        run_mp_token_authorize_do_apply, run_mp_token_authorize_preclaim,
        run_mp_token_authorize_preflight,
    };
    use protocol::{Ter, tfMPTUnauthorize, tfMPTUnauthorizeMask, trans_token};

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestCreateSink {
        owner_dir_page: Option<u64>,
        inserted: Vec<MPTokenAuthorizeCreateMutation<&'static str, &'static str>>,
    }

    impl MPTokenAuthorizeCreateSink<&'static str, &'static str> for TestCreateSink {
        fn insert_owner_dir(&mut self) -> Option<u64> {
            self.owner_dir_page
        }

        fn insert_mptoken(
            &mut self,
            mutation: MPTokenAuthorizeCreateMutation<&'static str, &'static str>,
        ) {
            self.inserted.push(mutation);
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestApplySink {
        result: Ter,
        calls: Vec<(&'static str, &'static str, u32, Option<&'static str>)>,
    }

    impl MPTokenAuthorizeApplySink<&'static str, &'static str> for TestApplySink {
        fn authorize_mptoken(
            &mut self,
            mpt_issuance_id: &'static str,
            account: &'static str,
            tx_flags: u32,
            holder: Option<&'static str>,
        ) -> Ter {
            self.calls
                .push((mpt_issuance_id, account, tx_flags, holder));
            self.result
        }
    }

    #[test]
    fn mp_token_authorize_mask_and_preflight_match_cpp() {
        assert_eq!(get_mp_token_authorize_flags_mask(), tfMPTUnauthorizeMask);
        assert_eq!(
            run_mp_token_authorize_preflight(MPTokenAuthorizePreflightFacts {
                account_equals_holder: true,
            }),
            Ter::TEM_MALFORMED
        );
        assert_eq!(
            run_mp_token_authorize_preflight(MPTokenAuthorizePreflightFacts {
                account_equals_holder: false,
            }),
            Ter::TES_SUCCESS
        );
    }

    #[test]
    fn mp_token_authorize_preclaim_matches_holder_flow_guards() {
        let missing = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
            holder_present: false,
            account_token_exists: false,
            tx_flags: tfMPTUnauthorize,
            token_balance_is_zero: true,
            token_locked_amount_is_zero: true,
            issuance_exists: true,
            single_asset_vault_enabled: false,
            token_locked: false,
            account_is_issuer: false,
            holder_account_exists: true,
            issuance_requires_auth: true,
            holder_token_exists: true,
            holder_is_pseudo_account: false,
        });
        let obligations = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
            account_token_exists: true,
            token_balance_is_zero: false,
            ..MPTokenAuthorizePreclaimFacts {
                holder_present: false,
                account_token_exists: true,
                tx_flags: tfMPTUnauthorize,
                token_balance_is_zero: true,
                token_locked_amount_is_zero: true,
                issuance_exists: true,
                single_asset_vault_enabled: false,
                token_locked: false,
                account_is_issuer: false,
                holder_account_exists: true,
                issuance_requires_auth: true,
                holder_token_exists: true,
                holder_is_pseudo_account: false,
            }
        });
        let locked = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
            holder_present: false,
            account_token_exists: true,
            tx_flags: tfMPTUnauthorize,
            token_balance_is_zero: true,
            token_locked_amount_is_zero: true,
            issuance_exists: true,
            single_asset_vault_enabled: true,
            token_locked: true,
            account_is_issuer: false,
            holder_account_exists: true,
            issuance_requires_auth: true,
            holder_token_exists: true,
            holder_is_pseudo_account: false,
        });
        let duplicate = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
            holder_present: false,
            account_token_exists: true,
            tx_flags: 0,
            token_balance_is_zero: true,
            token_locked_amount_is_zero: true,
            issuance_exists: true,
            single_asset_vault_enabled: false,
            token_locked: false,
            account_is_issuer: false,
            holder_account_exists: true,
            issuance_requires_auth: true,
            holder_token_exists: true,
            holder_is_pseudo_account: false,
        });

        assert_eq!(missing, Ter::TEC_OBJECT_NOT_FOUND);
        assert_eq!(obligations, Ter::TEC_HAS_OBLIGATIONS);
        assert_eq!(locked, Ter::TEC_NO_PERMISSION);
        assert_eq!(duplicate, Ter::TEC_DUPLICATE);
    }

    #[test]
    fn mp_token_authorize_preclaim_matches_issuer_flow_guards() {
        let no_dst = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
            holder_present: true,
            account_token_exists: false,
            tx_flags: 0,
            token_balance_is_zero: true,
            token_locked_amount_is_zero: true,
            issuance_exists: true,
            single_asset_vault_enabled: false,
            token_locked: false,
            account_is_issuer: true,
            holder_account_exists: false,
            issuance_requires_auth: true,
            holder_token_exists: true,
            holder_is_pseudo_account: false,
        });
        let no_auth = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
            holder_present: true,
            account_token_exists: false,
            tx_flags: 0,
            token_balance_is_zero: true,
            token_locked_amount_is_zero: true,
            issuance_exists: true,
            single_asset_vault_enabled: false,
            token_locked: false,
            account_is_issuer: true,
            holder_account_exists: true,
            issuance_requires_auth: false,
            holder_token_exists: true,
            holder_is_pseudo_account: false,
        });
        let pseudo = run_mp_token_authorize_preclaim(MPTokenAuthorizePreclaimFacts {
            holder_present: true,
            account_token_exists: false,
            tx_flags: 0,
            token_balance_is_zero: true,
            token_locked_amount_is_zero: true,
            issuance_exists: true,
            single_asset_vault_enabled: false,
            token_locked: false,
            account_is_issuer: true,
            holder_account_exists: true,
            issuance_requires_auth: true,
            holder_token_exists: true,
            holder_is_pseudo_account: true,
        });

        assert_eq!(no_dst, Ter::TEC_NO_DST);
        assert_eq!(no_auth, Ter::TEC_NO_AUTH);
        assert_eq!(pseudo, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn mp_token_authorize_create_owner_dir_order() {
        let mut sink = TestCreateSink {
            owner_dir_page: Some(17),
            inserted: Vec::new(),
        };

        let result = run_mp_token_authorize_create_mptoken("mpt", "alice", 9, &mut sink);

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            sink.inserted,
            vec![MPTokenAuthorizeCreateMutation {
                account: "alice",
                mpt_issuance_id: "mpt",
                flags: 9,
                owner_node: 17,
            }]
        );

        let mut missing_page = TestCreateSink {
            owner_dir_page: None,
            inserted: Vec::new(),
        };
        let dir_full = run_mp_token_authorize_create_mptoken("mpt", "alice", 9, &mut missing_page);
        assert_eq!(dir_full, Ter::TEC_DIR_FULL);
        assert_eq!(trans_token(dir_full), "tecDIR_FULL");
    }

    #[test]
    fn mp_token_authorize_do_apply_is_thin_wrapper() {
        let mut sink = TestApplySink {
            result: Ter::TES_SUCCESS,
            calls: Vec::new(),
        };

        let result = run_mp_token_authorize_do_apply(
            "mpt",
            "issuer",
            tfMPTUnauthorize,
            Some("holder"),
            &mut sink,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            sink.calls,
            vec![("mpt", "issuer", tfMPTUnauthorize, Some("holder"))]
        );
    }
}
