//! Typed `Keylet` helpers for `xrpl/protocol`.
//!
//! This module carries the `Keylet` pair of ledger-entry type plus key for the
//! migrated index helpers, together with the `ltANY` / `ltCHILD` check
//! semantics.

use basics::base_uint::{Uint160, Uint256};
use sha2::{Digest, Sha512};

use crate::{AccountID, Asset, Book, Currency, Issue, MPTID, SeqProxy, is_consistent_book};

const LEDGER_NAMESPACE_ACCOUNT: u16 = b'a' as u16;
const LEDGER_NAMESPACE_BOOK_DIR: u16 = b'B' as u16;
const LEDGER_NAMESPACE_DIR_NODE: u16 = b'd' as u16;
const LEDGER_NAMESPACE_TRUST_LINE: u16 = b'r' as u16;
const LEDGER_NAMESPACE_OWNER_DIR: u16 = b'O' as u16;
const LEDGER_NAMESPACE_SKIP_LIST: u16 = b's' as u16;
const LEDGER_NAMESPACE_ESCROW: u16 = b'u' as u16;
const LEDGER_NAMESPACE_AMENDMENTS: u16 = b'f' as u16;
const LEDGER_NAMESPACE_FEE_SETTINGS: u16 = b'e' as u16;
const LEDGER_NAMESPACE_OFFER: u16 = b'o' as u16;
const LEDGER_NAMESPACE_TICKET: u16 = b'T' as u16;
const LEDGER_NAMESPACE_SIGNER_LIST: u16 = b'S' as u16;
const LEDGER_NAMESPACE_PAYMENT_CHANNEL: u16 = b'x' as u16;
const LEDGER_NAMESPACE_CHECK: u16 = b'C' as u16;
const LEDGER_NAMESPACE_DEPOSIT_PREAUTH: u16 = b'p' as u16;
const LEDGER_NAMESPACE_DEPOSIT_PREAUTH_CREDENTIALS: u16 = b'P' as u16;
const LEDGER_NAMESPACE_NEGATIVE_UNL: u16 = b'N' as u16;
const LEDGER_NAMESPACE_NFTOKEN_OFFER: u16 = b'q' as u16;
const LEDGER_NAMESPACE_DID: u16 = b'I' as u16;
const LEDGER_NAMESPACE_ORACLE: u16 = b'R' as u16;
const LEDGER_NAMESPACE_NFTOKEN_BUY_OFFERS: u16 = b'h' as u16;
const LEDGER_NAMESPACE_NFTOKEN_SELL_OFFERS: u16 = b'i' as u16;
const LEDGER_NAMESPACE_AMM: u16 = b'A' as u16;
const LEDGER_NAMESPACE_MPTOKEN_ISSUANCE: u16 = b'~' as u16;
const LEDGER_NAMESPACE_MPTOKEN: u16 = b't' as u16;
const LEDGER_NAMESPACE_CREDENTIAL: u16 = b'D' as u16;
const LEDGER_NAMESPACE_PERMISSIONED_DOMAIN: u16 = b'm' as u16;
const LEDGER_NAMESPACE_DELEGATE: u16 = b'E' as u16;
const LEDGER_NAMESPACE_BRIDGE: u16 = b'H' as u16;
const LEDGER_NAMESPACE_XCHAIN_CLAIM_ID: u16 = b'Q' as u16;
const LEDGER_NAMESPACE_XCHAIN_CREATE_ACCOUNT_CLAIM_ID: u16 = b'K' as u16;
const LEDGER_NAMESPACE_VAULT: u16 = b'V' as u16;
const LEDGER_NAMESPACE_LOAN_BROKER: u16 = b'l' as u16;
const LEDGER_NAMESPACE_LOAN: u16 = b'L' as u16;
const BOOK_BASE_ISSUE_MPT_TAG: [u8; 1] = [0x01];
const BOOK_BASE_MPT_ISSUE_TAG: [u8; 1] = [0x02];
const BOOK_BASE_MPT_MPT_TAG: [u8; 1] = [0x03];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum LedgerEntryType {
    Any = 0x0000,
    NFTokenOffer = 0x0037,
    Check = 0x0043,
    DID = 0x0049,
    NegativeUnl = 0x004E,
    NFTokenPage = 0x0050,
    SignerList = 0x0053,
    Ticket = 0x0054,
    AccountRoot = 0x0061,
    Contract = 0x0063,
    DirectoryNode = 0x0064,
    Amendments = 0x0066,
    GeneratorMap = 0x0067,
    LedgerHashes = 0x0068,
    Bridge = 0x0069,
    Nickname = 0x006E,
    Offer = 0x006F,
    DepositPreauth = 0x0070,
    XChainOwnedClaimId = 0x0071,
    RippleState = 0x0072,
    FeeSettings = 0x0073,
    XChainOwnedCreateAccountClaimId = 0x0074,
    Escrow = 0x0075,
    PayChannel = 0x0078,
    AMM = 0x0079,
    MPTokenIssuance = 0x007E,
    MPToken = 0x007F,
    Oracle = 0x0080,
    Credential = 0x0081,
    PermissionedDomain = 0x0082,
    Delegate = 0x0083,
    Vault = 0x0084,
    LoanBroker = 0x0088,
    Loan = 0x0089,
    Child = 0x1CD2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LedgerEntryTypeInfo {
    pub entry_type: LedgerEntryType,
    pub code: u16,
    pub name: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct DirectAccountKeyletDesc {
    pub function: fn(Uint160) -> Keylet,
    pub expected_le_name: &'static str,
    pub include_in_tests: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keylet {
    pub key: Uint256,
    pub entry_type: LedgerEntryType,
}

impl Keylet {
    pub const fn new(entry_type: LedgerEntryType, key: Uint256) -> Self {
        Self { key, entry_type }
    }

    pub fn check_entry_type(self, actual_type: LedgerEntryType) -> bool {
        match self.entry_type {
            LedgerEntryType::Any => true,
            LedgerEntryType::Child => actual_type != LedgerEntryType::DirectoryNode,
            expected => expected as u16 == actual_type as u16,
        }
    }

    /// Mirrors the reference `Keylet::check(STLedgerEntry const&)` entry-level
    /// matching behavior.
    pub fn check_ledger_entry(self, actual_type: LedgerEntryType, actual_key: Uint256) -> bool {
        debug_assert!(
            !matches!(actual_type, LedgerEntryType::Any | LedgerEntryType::Child),
            "Keylet::check expects a concrete ledger entry type"
        );

        match self.entry_type {
            LedgerEntryType::Any => true,
            LedgerEntryType::Child => actual_type != LedgerEntryType::DirectoryNode,
            expected => expected == actual_type && self.key == actual_key,
        }
    }
}

impl LedgerEntryType {
    pub const fn code(self) -> u16 {
        self as u16
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            LedgerEntryType::Any => "Any",
            LedgerEntryType::NFTokenOffer => "NFTokenOffer",
            LedgerEntryType::Check => "Check",
            LedgerEntryType::DID => "DID",
            LedgerEntryType::NegativeUnl => "NegativeUNL",
            LedgerEntryType::NFTokenPage => "NFTokenPage",
            LedgerEntryType::SignerList => "SignerList",
            LedgerEntryType::Ticket => "Ticket",
            LedgerEntryType::AccountRoot => "AccountRoot",
            LedgerEntryType::Contract => "Contract",
            LedgerEntryType::DirectoryNode => "DirectoryNode",
            LedgerEntryType::Amendments => "Amendments",
            LedgerEntryType::GeneratorMap => "GeneratorMap",
            LedgerEntryType::LedgerHashes => "LedgerHashes",
            LedgerEntryType::Bridge => "Bridge",
            LedgerEntryType::Nickname => "Nickname",
            LedgerEntryType::Offer => "Offer",
            LedgerEntryType::DepositPreauth => "DepositPreauth",
            LedgerEntryType::XChainOwnedClaimId => "XChainOwnedClaimID",
            LedgerEntryType::RippleState => "RippleState",
            LedgerEntryType::FeeSettings => "FeeSettings",
            LedgerEntryType::XChainOwnedCreateAccountClaimId => "XChainOwnedCreateAccountClaimID",
            LedgerEntryType::Escrow => "Escrow",
            LedgerEntryType::PayChannel => "PayChannel",
            LedgerEntryType::AMM => "AMM",
            LedgerEntryType::MPTokenIssuance => "MPTokenIssuance",
            LedgerEntryType::MPToken => "MPToken",
            LedgerEntryType::Oracle => "Oracle",
            LedgerEntryType::Credential => "Credential",
            LedgerEntryType::PermissionedDomain => "PermissionedDomain",
            LedgerEntryType::Delegate => "Delegate",
            LedgerEntryType::Vault => "Vault",
            LedgerEntryType::LoanBroker => "LoanBroker",
            LedgerEntryType::Loan => "Loan",
            LedgerEntryType::Child => "Child",
        }
    }
}

pub const fn ledger_entry_type_code(entry_type: LedgerEntryType) -> u16 {
    entry_type.code()
}

pub fn ledger_entry_type_from_code(code: u16) -> Option<LedgerEntryType> {
    match code {
        0x0000 => Some(LedgerEntryType::Any),
        0x0037 => Some(LedgerEntryType::NFTokenOffer),
        0x0043 => Some(LedgerEntryType::Check),
        0x0049 => Some(LedgerEntryType::DID),
        0x004E => Some(LedgerEntryType::NegativeUnl),
        0x0050 => Some(LedgerEntryType::NFTokenPage),
        0x0053 => Some(LedgerEntryType::SignerList),
        0x0054 => Some(LedgerEntryType::Ticket),
        0x0061 => Some(LedgerEntryType::AccountRoot),
        0x0063 => Some(LedgerEntryType::Contract),
        0x0064 => Some(LedgerEntryType::DirectoryNode),
        0x0066 => Some(LedgerEntryType::Amendments),
        0x0067 => Some(LedgerEntryType::GeneratorMap),
        0x0068 => Some(LedgerEntryType::LedgerHashes),
        0x0069 => Some(LedgerEntryType::Bridge),
        0x006E => Some(LedgerEntryType::Nickname),
        0x006F => Some(LedgerEntryType::Offer),
        0x0070 => Some(LedgerEntryType::DepositPreauth),
        0x0071 => Some(LedgerEntryType::XChainOwnedClaimId),
        0x0072 => Some(LedgerEntryType::RippleState),
        0x0073 => Some(LedgerEntryType::FeeSettings),
        0x0074 => Some(LedgerEntryType::XChainOwnedCreateAccountClaimId),
        0x0075 => Some(LedgerEntryType::Escrow),
        0x0078 => Some(LedgerEntryType::PayChannel),
        0x0079 => Some(LedgerEntryType::AMM),
        0x007E => Some(LedgerEntryType::MPTokenIssuance),
        0x007F => Some(LedgerEntryType::MPToken),
        0x0080 => Some(LedgerEntryType::Oracle),
        0x0081 => Some(LedgerEntryType::Credential),
        0x0082 => Some(LedgerEntryType::PermissionedDomain),
        0x0083 => Some(LedgerEntryType::Delegate),
        0x0084 => Some(LedgerEntryType::Vault),
        0x0088 => Some(LedgerEntryType::LoanBroker),
        0x0089 => Some(LedgerEntryType::Loan),
        0x1CD2 => Some(LedgerEntryType::Child),
        _ => None,
    }
}

pub fn ledger_entry_type_from_name(name: &str) -> Option<LedgerEntryType> {
    match name {
        "Any" => Some(LedgerEntryType::Any),
        "NFTokenOffer" => Some(LedgerEntryType::NFTokenOffer),
        "Check" => Some(LedgerEntryType::Check),
        "DID" => Some(LedgerEntryType::DID),
        "NegativeUNL" => Some(LedgerEntryType::NegativeUnl),
        "NFTokenPage" => Some(LedgerEntryType::NFTokenPage),
        "SignerList" => Some(LedgerEntryType::SignerList),
        "Ticket" => Some(LedgerEntryType::Ticket),
        "AccountRoot" => Some(LedgerEntryType::AccountRoot),
        "Contract" => Some(LedgerEntryType::Contract),
        "DirectoryNode" => Some(LedgerEntryType::DirectoryNode),
        "Amendments" => Some(LedgerEntryType::Amendments),
        "GeneratorMap" => Some(LedgerEntryType::GeneratorMap),
        "LedgerHashes" => Some(LedgerEntryType::LedgerHashes),
        "Bridge" => Some(LedgerEntryType::Bridge),
        "Nickname" => Some(LedgerEntryType::Nickname),
        "Offer" => Some(LedgerEntryType::Offer),
        "DepositPreauth" => Some(LedgerEntryType::DepositPreauth),
        "XChainOwnedClaimID" => Some(LedgerEntryType::XChainOwnedClaimId),
        "RippleState" => Some(LedgerEntryType::RippleState),
        "FeeSettings" => Some(LedgerEntryType::FeeSettings),
        "XChainOwnedCreateAccountClaimID" => Some(LedgerEntryType::XChainOwnedCreateAccountClaimId),
        "Escrow" => Some(LedgerEntryType::Escrow),
        "PayChannel" => Some(LedgerEntryType::PayChannel),
        "AMM" => Some(LedgerEntryType::AMM),
        "MPTokenIssuance" => Some(LedgerEntryType::MPTokenIssuance),
        "MPToken" => Some(LedgerEntryType::MPToken),
        "Oracle" => Some(LedgerEntryType::Oracle),
        "Credential" => Some(LedgerEntryType::Credential),
        "PermissionedDomain" => Some(LedgerEntryType::PermissionedDomain),
        "Delegate" => Some(LedgerEntryType::Delegate),
        "Vault" => Some(LedgerEntryType::Vault),
        "LoanBroker" => Some(LedgerEntryType::LoanBroker),
        "Loan" => Some(LedgerEntryType::Loan),
        "Child" => Some(LedgerEntryType::Child),
        _ => None,
    }
}

const LEDGER_ENTRY_TYPE_CATALOG: &[LedgerEntryTypeInfo] = &[
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Any,
        code: 0x0000,
        name: "Any",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::NFTokenOffer,
        code: 0x0037,
        name: "NFTokenOffer",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Check,
        code: 0x0043,
        name: "Check",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::DID,
        code: 0x0049,
        name: "DID",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::NegativeUnl,
        code: 0x004E,
        name: "NegativeUNL",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::NFTokenPage,
        code: 0x0050,
        name: "NFTokenPage",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::SignerList,
        code: 0x0053,
        name: "SignerList",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Ticket,
        code: 0x0054,
        name: "Ticket",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::AccountRoot,
        code: 0x0061,
        name: "AccountRoot",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Contract,
        code: 0x0063,
        name: "Contract",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::DirectoryNode,
        code: 0x0064,
        name: "DirectoryNode",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Amendments,
        code: 0x0066,
        name: "Amendments",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::GeneratorMap,
        code: 0x0067,
        name: "GeneratorMap",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::LedgerHashes,
        code: 0x0068,
        name: "LedgerHashes",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Bridge,
        code: 0x0069,
        name: "Bridge",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Nickname,
        code: 0x006E,
        name: "Nickname",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Offer,
        code: 0x006F,
        name: "Offer",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::DepositPreauth,
        code: 0x0070,
        name: "DepositPreauth",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::XChainOwnedClaimId,
        code: 0x0071,
        name: "XChainOwnedClaimID",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::RippleState,
        code: 0x0072,
        name: "RippleState",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::FeeSettings,
        code: 0x0073,
        name: "FeeSettings",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::XChainOwnedCreateAccountClaimId,
        code: 0x0074,
        name: "XChainOwnedCreateAccountClaimID",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Escrow,
        code: 0x0075,
        name: "Escrow",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::PayChannel,
        code: 0x0078,
        name: "PayChannel",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::AMM,
        code: 0x0079,
        name: "AMM",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::MPTokenIssuance,
        code: 0x007E,
        name: "MPTokenIssuance",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::MPToken,
        code: 0x007F,
        name: "MPToken",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Oracle,
        code: 0x0080,
        name: "Oracle",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Credential,
        code: 0x0081,
        name: "Credential",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::PermissionedDomain,
        code: 0x0082,
        name: "PermissionedDomain",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Delegate,
        code: 0x0083,
        name: "Delegate",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Vault,
        code: 0x0084,
        name: "Vault",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::LoanBroker,
        code: 0x0088,
        name: "LoanBroker",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Loan,
        code: 0x0089,
        name: "Loan",
    },
    LedgerEntryTypeInfo {
        entry_type: LedgerEntryType::Child,
        code: 0x1CD2,
        name: "Child",
    },
];

pub fn ledger_entry_type_catalog() -> &'static [LedgerEntryTypeInfo] {
    LEDGER_ENTRY_TYPE_CATALOG
}

pub fn mpt_id_to_uint192(mptid: Uint256) -> MPTID {
    MPTID::from_slice(&mptid.data()[0..24]).expect("uint192 from mptid slice")
}

pub fn account_keylet(account_id: Uint160) -> Keylet {
    Keylet::new(LedgerEntryType::AccountRoot, account_root_key(account_id))
}

pub fn line(account0: AccountID, account1: AccountID, currency: Currency) -> Keylet {
    let (first, second) = if account0 <= account1 {
        (account0, account1)
    } else {
        (account1, account0)
    };

    Keylet::new(
        LedgerEntryType::RippleState,
        index_hash_with_slices(
            LEDGER_NAMESPACE_TRUST_LINE,
            &[first.data(), second.data(), currency.data()],
        ),
    )
}

pub fn line_from_issue(account: AccountID, issue: Issue) -> Keylet {
    line(account, issue.account, issue.currency)
}

pub fn child_keylet(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Child, key)
}

pub fn ledger_hashes_keylet() -> Keylet {
    Keylet::new(LedgerEntryType::LedgerHashes, skip_key())
}

pub fn skip_keylet() -> Keylet {
    ledger_hashes_keylet()
}

pub fn skip_keylet_for_ledger(ledger_seq: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::LedgerHashes,
        index_hash_with_u32(LEDGER_NAMESPACE_SKIP_LIST, ledger_seq >> 16),
    )
}

pub fn amendments_keylet() -> Keylet {
    Keylet::new(LedgerEntryType::Amendments, amendments_key())
}

pub fn fee_settings_keylet() -> Keylet {
    Keylet::new(LedgerEntryType::FeeSettings, fees_key())
}

pub fn negative_unl_keylet() -> Keylet {
    Keylet::new(
        LedgerEntryType::NegativeUnl,
        singleton_key(LEDGER_NAMESPACE_NEGATIVE_UNL),
    )
}

pub fn nft_offer_keylet(offer: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::NFTokenOffer, offer)
}

pub fn nft_offer_keylet_from_key(offer: Uint256) -> Keylet {
    nft_offer_keylet(offer)
}

pub fn nft_offer_keylet_for_owner(owner: Uint160, sequence: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::NFTokenOffer,
        index_hash_with_slices(
            LEDGER_NAMESPACE_NFTOKEN_OFFER,
            &[owner.data(), &sequence.to_be_bytes()],
        ),
    )
}

pub fn nft_buy_offers_keylet(token_id: Uint256) -> Keylet {
    Keylet::new(
        LedgerEntryType::DirectoryNode,
        index_hash_with_bytes(LEDGER_NAMESPACE_NFTOKEN_BUY_OFFERS, token_id.data()),
    )
}

pub fn nft_sell_offers_keylet(token_id: Uint256) -> Keylet {
    Keylet::new(
        LedgerEntryType::DirectoryNode,
        index_hash_with_bytes(LEDGER_NAMESPACE_NFTOKEN_SELL_OFFERS, token_id.data()),
    )
}

pub fn nft_page_min_keylet(owner: Uint160) -> Keylet {
    Keylet::new(LedgerEntryType::NFTokenPage, nft_page_min(owner))
}

pub fn nft_page_max_keylet(owner: Uint160) -> Keylet {
    Keylet::new(LedgerEntryType::NFTokenPage, nft_page_max(owner))
}

pub fn nft_page_keylet(base: Keylet, token: Uint256) -> Keylet {
    assert!(
        matches!(base.entry_type, LedgerEntryType::NFTokenPage),
        "nft_page_keylet requires an NFTokenPage base"
    );
    Keylet::new(LedgerEntryType::NFTokenPage, nft_page(base, token))
}

pub fn owner_dir_keylet(account_id: Uint160) -> Keylet {
    Keylet::new(LedgerEntryType::DirectoryNode, owner_dir_key(account_id))
}

pub fn get_book_base(book: Book) -> Uint256 {
    assert!(
        is_consistent_book(book),
        "get_book_base requires a consistent Book"
    );

    let index = match (book.r#in, book.out, book.domain) {
        (Asset::Issue(input), Asset::Issue(output), Some(domain)) => index_hash_with_slices(
            LEDGER_NAMESPACE_BOOK_DIR,
            &[
                input.currency.data(),
                output.currency.data(),
                input.account.data(),
                output.account.data(),
                domain.data(),
            ],
        ),
        (Asset::Issue(input), Asset::Issue(output), None) => index_hash_with_slices(
            LEDGER_NAMESPACE_BOOK_DIR,
            &[
                input.currency.data(),
                output.currency.data(),
                input.account.data(),
                output.account.data(),
            ],
        ),
        (Asset::Issue(input), Asset::MPTIssue(output), Some(domain)) => index_hash_with_slices(
            LEDGER_NAMESPACE_BOOK_DIR,
            &[
                &BOOK_BASE_ISSUE_MPT_TAG,
                input.currency.data(),
                output.mpt_id().data(),
                input.account.data(),
                domain.data(),
            ],
        ),
        (Asset::Issue(input), Asset::MPTIssue(output), None) => index_hash_with_slices(
            LEDGER_NAMESPACE_BOOK_DIR,
            &[
                &BOOK_BASE_ISSUE_MPT_TAG,
                input.currency.data(),
                output.mpt_id().data(),
                input.account.data(),
            ],
        ),
        (Asset::MPTIssue(input), Asset::Issue(output), Some(domain)) => index_hash_with_slices(
            LEDGER_NAMESPACE_BOOK_DIR,
            &[
                &BOOK_BASE_MPT_ISSUE_TAG,
                input.mpt_id().data(),
                output.currency.data(),
                output.account.data(),
                domain.data(),
            ],
        ),
        (Asset::MPTIssue(input), Asset::Issue(output), None) => index_hash_with_slices(
            LEDGER_NAMESPACE_BOOK_DIR,
            &[
                &BOOK_BASE_MPT_ISSUE_TAG,
                input.mpt_id().data(),
                output.currency.data(),
                output.account.data(),
            ],
        ),
        (Asset::MPTIssue(input), Asset::MPTIssue(output), Some(domain)) => index_hash_with_slices(
            LEDGER_NAMESPACE_BOOK_DIR,
            &[
                &BOOK_BASE_MPT_MPT_TAG,
                input.mpt_id().data(),
                output.mpt_id().data(),
                domain.data(),
            ],
        ),
        (Asset::MPTIssue(input), Asset::MPTIssue(output), None) => index_hash_with_slices(
            LEDGER_NAMESPACE_BOOK_DIR,
            &[
                &BOOK_BASE_MPT_MPT_TAG,
                input.mpt_id().data(),
                output.mpt_id().data(),
            ],
        ),
    };

    quality_keylet(Keylet::new(LedgerEntryType::DirectoryNode, index), 0).key
}

pub fn book_keylet(book: Book) -> Keylet {
    Keylet::new(LedgerEntryType::DirectoryNode, get_book_base(book))
}

pub fn directory_node_keylet(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::DirectoryNode, key)
}

pub fn quality_keylet(base: Keylet, quality: u64) -> Keylet {
    assert!(
        matches!(base.entry_type, LedgerEntryType::DirectoryNode),
        "quality_keylet requires a DirectoryNode base"
    );
    Keylet::new(
        LedgerEntryType::DirectoryNode,
        directory_quality_key(base.key, quality),
    )
}

pub fn next_keylet(base: Keylet) -> Keylet {
    assert!(
        matches!(base.entry_type, LedgerEntryType::DirectoryNode),
        "next_keylet requires a DirectoryNode base"
    );
    Keylet::new(LedgerEntryType::DirectoryNode, next_quality_key(base.key))
}

pub fn page_keylet(root: Keylet, index: u64) -> Keylet {
    assert!(
        matches!(root.entry_type, LedgerEntryType::DirectoryNode),
        "page_keylet requires a DirectoryNode root"
    );
    if index == 0 {
        return root;
    }

    Keylet::new(
        LedgerEntryType::DirectoryNode,
        index_hash_with_slices(
            LEDGER_NAMESPACE_DIR_NODE,
            &[root.key.data(), &index.to_be_bytes()],
        ),
    )
}

pub fn quality_from_key(index: Uint256) -> u64 {
    u64::from_be_bytes(
        index.data()[24..]
            .try_into()
            .expect("directory quality suffix must contain 8 bytes"),
    )
}

pub fn offer_keylet(account_id: Uint160, sequence: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::Offer,
        index_hash_with_slices(
            LEDGER_NAMESPACE_OFFER,
            &[account_id.data(), &sequence.to_be_bytes()],
        ),
    )
}

pub fn offer_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Offer, key)
}

pub fn ticket_keylet(account_id: Uint160, ticket_seq: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::Ticket,
        index_hash_with_slices(
            LEDGER_NAMESPACE_TICKET,
            &[account_id.data(), &ticket_seq.to_be_bytes()],
        ),
    )
}

pub fn ticket_keylet_from_seq_proxy(account_id: Uint160, ticket_seq: SeqProxy) -> Keylet {
    assert!(
        ticket_seq.is_ticket(),
        "ticket_keylet_from_seq_proxy requires a ticket SeqProxy"
    );
    ticket_keylet(account_id, ticket_seq.value())
}

pub fn ticket_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Ticket, key)
}

pub fn ticket_index(account_id: Uint160, ticket_seq: u32) -> Uint256 {
    ticket_keylet(account_id, ticket_seq).key
}

pub fn ticket_index_from_seq_proxy(account_id: Uint160, ticket_seq: SeqProxy) -> Uint256 {
    ticket_keylet_from_seq_proxy(account_id, ticket_seq).key
}

pub fn signers_keylet(account_id: Uint160) -> Keylet {
    signers_keylet_for_page(account_id, 0)
}

pub fn signers_keylet_for_page(account_id: Uint160, page: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::SignerList,
        index_hash_with_slices(
            LEDGER_NAMESPACE_SIGNER_LIST,
            &[account_id.data(), &page.to_be_bytes()],
        ),
    )
}

pub fn check_keylet(account_id: Uint160, sequence: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::Check,
        index_hash_with_slices(
            LEDGER_NAMESPACE_CHECK,
            &[account_id.data(), &sequence.to_be_bytes()],
        ),
    )
}

pub fn check_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Check, key)
}

pub fn deposit_preauth_keylet(owner: Uint160, preauthorized: Uint160) -> Keylet {
    Keylet::new(
        LedgerEntryType::DepositPreauth,
        index_hash_with_slices(
            LEDGER_NAMESPACE_DEPOSIT_PREAUTH,
            &[owner.data(), preauthorized.data()],
        ),
    )
}

pub fn deposit_preauth_credentials_keylet(owner: Uint160, auth_creds: &[Uint256]) -> Keylet {
    Keylet::new(
        LedgerEntryType::DepositPreauth,
        index_hash_with_u256s(
            LEDGER_NAMESPACE_DEPOSIT_PREAUTH_CREDENTIALS,
            owner.data(),
            auth_creds,
        ),
    )
}

pub fn deposit_preauth_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::DepositPreauth, key)
}

pub fn unchecked_keylet(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Any, key)
}

pub fn escrow_keylet(source: Uint160, sequence: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::Escrow,
        index_hash_with_slices(
            LEDGER_NAMESPACE_ESCROW,
            &[source.data(), &sequence.to_be_bytes()],
        ),
    )
}

pub fn escrow_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Escrow, key)
}

pub fn pay_channel_keylet(source: Uint160, destination: Uint160, sequence: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::PayChannel,
        index_hash_with_slices(
            LEDGER_NAMESPACE_PAYMENT_CHANNEL,
            &[source.data(), destination.data(), &sequence.to_be_bytes()],
        ),
    )
}

pub fn pay_channel_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::PayChannel, key)
}

pub fn vault_keylet(owner: Uint160, sequence: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::Vault,
        index_hash_with_slices(
            LEDGER_NAMESPACE_VAULT,
            &[owner.data(), &sequence.to_be_bytes()],
        ),
    )
}

pub fn vault_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Vault, key)
}

pub fn loan_broker_keylet(owner: Uint160, sequence: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::LoanBroker,
        index_hash_with_slices(
            LEDGER_NAMESPACE_LOAN_BROKER,
            &[owner.data(), &sequence.to_be_bytes()],
        ),
    )
}

pub fn loan_broker_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::LoanBroker, key)
}

pub fn loan_keylet(loan_broker_id: Uint256, loan_seq: u32) -> Keylet {
    Keylet::new(LedgerEntryType::Loan, loan_key(loan_broker_id, loan_seq))
}

pub fn loan_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Loan, key)
}

pub fn did_keylet(account: Uint160) -> Keylet {
    Keylet::new(LedgerEntryType::DID, did_key(account))
}

pub fn oracle_keylet(account: Uint160, document_id: u32) -> Keylet {
    Keylet::new(LedgerEntryType::Oracle, oracle_key(account, document_id))
}

pub fn credential_keylet(subject: Uint160, issuer: Uint160, cred_type: &[u8]) -> Keylet {
    Keylet::new(
        LedgerEntryType::Credential,
        index_hash_with_slices(
            LEDGER_NAMESPACE_CREDENTIAL,
            &[subject.data(), issuer.data(), cred_type],
        ),
    )
}

pub fn credential_keylet_from_key(key: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Credential, key)
}

pub fn mpt_issuance_keylet(seq: u32, issuer: Uint160) -> Keylet {
    Keylet::new(
        LedgerEntryType::MPTokenIssuance,
        mpt_issuance_id(seq, issuer),
    )
}

pub fn mpt_issuance_keylet_from_mptid(issuance_id: MPTID) -> Keylet {
    Keylet::new(
        LedgerEntryType::MPTokenIssuance,
        index_hash_with_bytes(LEDGER_NAMESPACE_MPTOKEN_ISSUANCE, issuance_id.data()),
    )
}

pub fn mpt_issuance_keylet_from_id(issuance_id: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::MPTokenIssuance, issuance_id)
}

pub fn mptoken_keylet(issuance_id: Uint256, holder: Uint160) -> Keylet {
    Keylet::new(
        LedgerEntryType::MPToken,
        index_hash_with_slices(
            LEDGER_NAMESPACE_MPTOKEN,
            &[issuance_id.data(), holder.data()],
        ),
    )
}

pub fn mptoken_keylet_from_mptid(issuance_id: MPTID, holder: Uint160) -> Keylet {
    mptoken_keylet(mpt_issuance_keylet_from_mptid(issuance_id).key, holder)
}

pub fn mptoken_keylet_from_id(issuance_id: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::MPToken, issuance_id)
}

pub fn permissioned_domain_keylet(account: Uint160, seq: u32) -> Keylet {
    Keylet::new(
        LedgerEntryType::PermissionedDomain,
        index_hash_with_slices(
            LEDGER_NAMESPACE_PERMISSIONED_DOMAIN,
            &[account.data(), &seq.to_be_bytes()],
        ),
    )
}

pub fn permissioned_domain_keylet_from_id(domain_id: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::PermissionedDomain, domain_id)
}

pub fn delegate_keylet(account: Uint160, authorized_account: Uint160) -> Keylet {
    Keylet::new(
        LedgerEntryType::Delegate,
        index_hash_with_slices(
            LEDGER_NAMESPACE_DELEGATE,
            &[account.data(), authorized_account.data()],
        ),
    )
}

pub fn get_quality_next(index: Uint256) -> Uint256 {
    // This gives the start of the NEXT book's range, past all qualities in this book.
    let mut data = *index.data();
    // Increment byte 23 (with carry into bytes 22, 21, etc.)
    for i in (0..24).rev() {
        data[i] = data[i].wrapping_add(1);
        if data[i] != 0 {
            break;
        }
    }
    // Zero out the quality portion (bytes 24-31)
    data[24..].fill(0);
    Uint256::from_slice(&data).unwrap()
}

pub use account_keylet as account;
pub use amendments_keylet as amendments;
pub use book_keylet as book;
pub use check_keylet as check;
pub use credential_keylet as credential;
pub use delegate_keylet as delegate;
pub use deposit_preauth_keylet as depositPreauth;
pub use did_keylet as did;
pub use escrow_keylet as escrow;
pub use fee_settings_keylet as fees;
pub use get_book_base as getBookBase;
pub use get_quality_next as getQualityNext;
pub use loan_broker_keylet as loanbroker;
pub use mpt_issuance_keylet as mptIssuance;
pub use mptoken_keylet as mptoken;
pub use negative_unl_keylet as negativeUNL;
pub use next_keylet as next;
pub use offer_keylet as offer;
pub use oracle_keylet as oracle;
pub use owner_dir_keylet as ownerDir;
pub use page_keylet as page;
pub use pay_channel_keylet as payChan;
pub use permissioned_domain_keylet as permissionedDomain;
pub use quality_from_key as getQuality;
pub use quality_keylet as quality;
pub use signers_keylet as signers;
pub use skip_keylet as skip;
pub use ticket_index as getTicketIndex;
pub use ticket_keylet as ticket;
pub use unchecked_keylet as unchecked;
pub use vault_keylet as vault;

pub const DIRECT_ACCOUNT_KEYLETS: [DirectAccountKeyletDesc; 6] = [
    DirectAccountKeyletDesc {
        function: account_keylet,
        expected_le_name: "AccountRoot",
        include_in_tests: false,
    },
    DirectAccountKeyletDesc {
        function: owner_dir_keylet,
        expected_le_name: "DirectoryNode",
        include_in_tests: true,
    },
    DirectAccountKeyletDesc {
        function: signers_keylet,
        expected_le_name: "SignerList",
        include_in_tests: true,
    },
    DirectAccountKeyletDesc {
        function: nft_page_min_keylet,
        expected_le_name: "NFTokenPage",
        include_in_tests: true,
    },
    DirectAccountKeyletDesc {
        function: nft_page_max_keylet,
        expected_le_name: "NFTokenPage",
        include_in_tests: true,
    },
    DirectAccountKeyletDesc {
        function: did_keylet,
        expected_le_name: "DID",
        include_in_tests: true,
    },
];

pub fn amm(issue1: Asset, issue2: Asset) -> Keylet {
    let (left, right) = if issue1 <= issue2 {
        (issue1, issue2)
    } else {
        (issue2, issue1)
    };

    let key = match (left, right) {
        (Asset::Issue(left), Asset::Issue(right)) => index_hash_with_slices(
            LEDGER_NAMESPACE_AMM,
            &[
                left.account.data(),
                left.currency.data(),
                right.account.data(),
                right.currency.data(),
            ],
        ),
        (Asset::Issue(left), Asset::MPTIssue(right)) => index_hash_with_slices(
            LEDGER_NAMESPACE_AMM,
            &[
                left.account.data(),
                left.currency.data(),
                right.mpt_id().data(),
            ],
        ),
        (Asset::MPTIssue(left), Asset::Issue(right)) => index_hash_with_slices(
            LEDGER_NAMESPACE_AMM,
            &[
                left.mpt_id().data(),
                right.account.data(),
                right.currency.data(),
            ],
        ),
        (Asset::MPTIssue(left), Asset::MPTIssue(right)) => index_hash_with_slices(
            LEDGER_NAMESPACE_AMM,
            &[left.mpt_id().data(), right.mpt_id().data()],
        ),
    };

    Keylet::new(LedgerEntryType::AMM, key)
}

pub fn amm_keylet(id: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::AMM, id)
}

pub fn bridge_keylet(id: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::Bridge, id)
}

pub fn bridge_keylet_from_door_issue(door: Uint160, issue: Issue) -> Keylet {
    Keylet::new(
        LedgerEntryType::Bridge,
        index_hash_with_slices(
            LEDGER_NAMESPACE_BRIDGE,
            &[door.data(), issue.currency.data()],
        ),
    )
}

pub fn xchain_owned_claim_id_keylet(id: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::XChainOwnedClaimId, id)
}

pub fn xchain_owned_claim_id_keylet_from_bridge(
    locking_chain_door: Uint160,
    locking_chain_issue: Issue,
    issuing_chain_door: Uint160,
    issuing_chain_issue: Issue,
    seq: u64,
) -> Keylet {
    Keylet::new(
        LedgerEntryType::XChainOwnedClaimId,
        index_hash_with_slices(
            LEDGER_NAMESPACE_XCHAIN_CLAIM_ID,
            &[
                locking_chain_door.data(),
                locking_chain_issue.account.data(),
                locking_chain_issue.currency.data(),
                issuing_chain_door.data(),
                issuing_chain_issue.account.data(),
                issuing_chain_issue.currency.data(),
                &seq.to_be_bytes(),
            ],
        ),
    )
}

pub fn xchain_owned_create_account_claim_id_keylet(id: Uint256) -> Keylet {
    Keylet::new(LedgerEntryType::XChainOwnedCreateAccountClaimId, id)
}

pub fn xchain_owned_create_account_claim_id_keylet_from_bridge(
    locking_chain_door: Uint160,
    locking_chain_issue: Issue,
    issuing_chain_door: Uint160,
    issuing_chain_issue: Issue,
    seq: u64,
) -> Keylet {
    Keylet::new(
        LedgerEntryType::XChainOwnedCreateAccountClaimId,
        index_hash_with_slices(
            LEDGER_NAMESPACE_XCHAIN_CREATE_ACCOUNT_CLAIM_ID,
            &[
                locking_chain_door.data(),
                locking_chain_issue.account.data(),
                locking_chain_issue.currency.data(),
                issuing_chain_door.data(),
                issuing_chain_issue.account.data(),
                issuing_chain_issue.currency.data(),
                &seq.to_be_bytes(),
            ],
        ),
    )
}

pub fn account_root_key(account_id: Uint160) -> Uint256 {
    index_hash_with_bytes(LEDGER_NAMESPACE_ACCOUNT, account_id.data())
}

pub fn amendments_key() -> Uint256 {
    singleton_key(LEDGER_NAMESPACE_AMENDMENTS)
}

pub fn fees_key() -> Uint256 {
    singleton_key(LEDGER_NAMESPACE_FEE_SETTINGS)
}

pub fn loan_key(loan_broker_id: Uint256, loan_seq: u32) -> Uint256 {
    index_hash_with_slices(
        LEDGER_NAMESPACE_LOAN,
        &[loan_broker_id.data(), &loan_seq.to_be_bytes()],
    )
}

fn owner_dir_key(account_id: Uint160) -> Uint256 {
    index_hash_with_bytes(LEDGER_NAMESPACE_OWNER_DIR, account_id.data())
}

fn directory_quality_key(base: Uint256, quality: u64) -> Uint256 {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(base.data());
    bytes[24..].copy_from_slice(&quality.to_be_bytes());
    Uint256::from_array(bytes)
}

fn next_quality_key(base: Uint256) -> Uint256 {
    base + Uint256::from_hex("0000000000000000000000000000000000000000000000010000000000000000")
        .expect("expected getQualityNext increment constant should parse")
}

fn skip_key() -> Uint256 {
    singleton_key(LEDGER_NAMESPACE_SKIP_LIST)
}

fn did_key(account: Uint160) -> Uint256 {
    index_hash_with_bytes(LEDGER_NAMESPACE_DID, account.data())
}

fn oracle_key(account: Uint160, document_id: u32) -> Uint256 {
    index_hash_with_slices(
        LEDGER_NAMESPACE_ORACLE,
        &[account.data(), &document_id.to_be_bytes()],
    )
}

fn mpt_issuance_id(seq: u32, issuer: Uint160) -> Uint256 {
    let mut bytes = [0u8; 24];
    bytes[..4].copy_from_slice(&seq.to_be_bytes());
    bytes[4..].copy_from_slice(issuer.data());
    index_hash_with_bytes(LEDGER_NAMESPACE_MPTOKEN_ISSUANCE, &bytes)
}

fn nft_page_mask() -> Uint256 {
    Uint256::from_hex("0000000000000000000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF")
        .expect("expected nft page mask should parse")
}

fn nft_page_min(owner: Uint160) -> Uint256 {
    let mut bytes = [0u8; 32];
    bytes[..owner.data().len()].copy_from_slice(owner.data());
    Uint256::from_slice(&bytes).expect("nft page min should contain 32 bytes")
}

fn nft_page_max(owner: Uint160) -> Uint256 {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(nft_page_mask().data());
    bytes[..owner.data().len()].copy_from_slice(owner.data());
    Uint256::from_slice(&bytes).expect("nft page max should contain 32 bytes")
}

fn nft_page(base: Keylet, token: Uint256) -> Uint256 {
    let mask = nft_page_mask();
    (base.key & !mask) + (token & mask)
}

fn singleton_key(namespace: u16) -> Uint256 {
    index_hash_with_bytes(namespace, &[])
}

fn index_hash_with_u32(namespace: u16, value: u32) -> Uint256 {
    index_hash_with_slices(namespace, &[&value.to_be_bytes()])
}

fn index_hash_with_bytes(namespace: u16, bytes: &[u8]) -> Uint256 {
    index_hash_with_slices(namespace, &[bytes])
}

fn index_hash_with_slices(namespace: u16, slices: &[&[u8]]) -> Uint256 {
    let mut hasher = Sha512::new();
    hasher.update(namespace.to_be_bytes());
    for slice in slices {
        hasher.update(slice);
    }
    let digest = hasher.finalize();
    Uint256::from_slice(&digest[..32]).expect("SHA-512 half output must contain 32 bytes")
}

fn index_hash_with_u256s(namespace: u16, head: &[u8], values: &[Uint256]) -> Uint256 {
    let mut hasher = Sha512::new();
    hasher.update(namespace.to_be_bytes());
    hasher.update(head);
    for value in values {
        hasher.update(value.data());
    }
    hasher.update((values.len() as u64).to_be_bytes());
    let digest = hasher.finalize();
    Uint256::from_slice(&digest[..32]).expect("SHA-512 half output must contain 32 bytes")
}

#[cfg(test)]
mod tests {
    use super::{
        Keylet, LedgerEntryType, account_keylet, account_root_key, amendments_key,
        amendments_keylet, amm_keylet, bridge_keylet, check_keylet, check_keylet_from_key,
        credential_keylet, credential_keylet_from_key, delegate_keylet,
        deposit_preauth_credentials_keylet, deposit_preauth_keylet,
        deposit_preauth_keylet_from_key, did_keylet, directory_node_keylet, escrow_keylet,
        escrow_keylet_from_key, fee_settings_keylet, fees_key, ledger_hashes_keylet,
        loan_broker_keylet, loan_broker_keylet_from_key, loan_key, loan_keylet,
        loan_keylet_from_key, mpt_issuance_keylet, mpt_issuance_keylet_from_id, mptoken_keylet,
        mptoken_keylet_from_id, negative_unl_keylet, next_keylet, nft_buy_offers_keylet,
        nft_offer_keylet, nft_offer_keylet_for_owner, nft_offer_keylet_from_key, nft_page_keylet,
        nft_page_max_keylet, nft_page_min_keylet, nft_sell_offers_keylet, offer_keylet,
        offer_keylet_from_key, oracle_keylet, owner_dir_keylet, page_keylet, pay_channel_keylet,
        pay_channel_keylet_from_key, permissioned_domain_keylet,
        permissioned_domain_keylet_from_id, quality_from_key, quality_keylet, signers_keylet,
        signers_keylet_for_page, skip_keylet, skip_keylet_for_ledger, ticket_keylet,
        ticket_keylet_from_key, ticket_keylet_from_seq_proxy, unchecked_keylet, vault_keylet,
        vault_keylet_from_key, xchain_owned_claim_id_keylet,
        xchain_owned_create_account_claim_id_keylet,
    };
    use crate::SeqProxy;
    use basics::base_uint::{Uint160, Uint256};

    #[test]
    fn singleton_keys_match_current_cpp_hashes() {
        assert_eq!(
            amendments_key(),
            Uint256::from_hex("7DB0788C020F02780A673DC74757F23823FA3014C1866E72CC4CD8B226CD6EF4")
                .expect("expected amendments key should parse")
        );
        assert_eq!(
            fees_key(),
            Uint256::from_hex("4BC50C9B0D8515D3EAAE1E74B29A95804346C491EE1A95BF25E4AAB854A6A651")
                .expect("expected fees key should parse")
        );
    }

    #[test]
    fn account_root_key_matches_current_cpp_genesis_vector() {
        let genesis_account = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected genesis account id should parse");

        assert_eq!(
            account_root_key(genesis_account),
            Uint256::from_hex("2B6AC232AA4C4BE41BF49D2459FA4A0347E1B543A4C92FCEE0821C0201E2E9A8")
                .expect("expected account root key should parse")
        );
    }

    #[test]
    fn typed_keylets_reuse_expected_entry_types() {
        let account = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected account id should parse");

        assert_eq!(
            account_keylet(account).entry_type,
            LedgerEntryType::AccountRoot
        );
        assert_eq!(amendments_keylet().entry_type, LedgerEntryType::Amendments);
        assert_eq!(
            fee_settings_keylet().entry_type,
            LedgerEntryType::FeeSettings
        );
        assert_eq!(
            negative_unl_keylet().entry_type,
            LedgerEntryType::NegativeUnl
        );
        assert_eq!(
            owner_dir_keylet(account).entry_type,
            LedgerEntryType::DirectoryNode
        );
    }

    #[test]
    fn keylet_check_entry_type_matches_any_and_child_rules() {
        let key = Uint256::from_u64(9);

        assert!(unchecked_keylet(key).check_entry_type(LedgerEntryType::Offer));
        assert!(Keylet::new(LedgerEntryType::Child, key).check_entry_type(LedgerEntryType::Offer));
        assert!(
            !Keylet::new(LedgerEntryType::Child, key)
                .check_entry_type(LedgerEntryType::DirectoryNode)
        );
        assert!(account_keylet(Uint160::default()).check_entry_type(LedgerEntryType::AccountRoot));
        assert!(!account_keylet(Uint160::default()).check_entry_type(LedgerEntryType::Offer));
    }

    #[test]
    fn skip_keylet_uses_high_ledger_bits() {
        let low = skip_keylet_for_ledger(0x1234_0001);
        let high = skip_keylet_for_ledger(0x1234_FFFF);
        let different = skip_keylet_for_ledger(0x1235_0000);

        assert_eq!(skip_keylet().entry_type, LedgerEntryType::LedgerHashes);
        assert_eq!(low, high);
        assert_ne!(low, different);
        assert_eq!(ledger_hashes_keylet(), skip_keylet());
    }

    #[test]
    fn typed_keylet_catalog_matches_current_cpp_roles() {
        let account = Uint160::from_hex("1111111111111111111111111111111111111111")
            .expect("expected account should parse");
        let other = Uint160::from_hex("2222222222222222222222222222222222222222")
            .expect("expected other account should parse");
        let key = Uint256::from_u64(42);

        assert_eq!(
            nft_offer_keylet(key).entry_type,
            LedgerEntryType::NFTokenOffer
        );
        assert_eq!(nft_offer_keylet_from_key(key), nft_offer_keylet(key));
        assert_eq!(
            nft_buy_offers_keylet(key).entry_type,
            LedgerEntryType::DirectoryNode
        );
        assert_eq!(
            nft_sell_offers_keylet(key).entry_type,
            LedgerEntryType::DirectoryNode
        );
        assert_eq!(did_keylet(account).entry_type, LedgerEntryType::DID);
        assert_eq!(
            oracle_keylet(account, 7).entry_type,
            LedgerEntryType::Oracle
        );
        assert_eq!(
            delegate_keylet(account, other).entry_type,
            LedgerEntryType::Delegate
        );
        assert_eq!(amm_keylet(key).entry_type, LedgerEntryType::AMM);
        assert_eq!(bridge_keylet(key).entry_type, LedgerEntryType::Bridge);
        assert_eq!(
            xchain_owned_claim_id_keylet(key).entry_type,
            LedgerEntryType::XChainOwnedClaimId
        );
        assert_eq!(
            xchain_owned_create_account_claim_id_keylet(key).entry_type,
            LedgerEntryType::XChainOwnedCreateAccountClaimId
        );

        assert_eq!(
            credential_keylet(account, other, b"cred").entry_type,
            LedgerEntryType::Credential
        );
        assert_eq!(
            credential_keylet_from_key(key),
            Keylet::new(LedgerEntryType::Credential, key)
        );
        assert_eq!(
            mpt_issuance_keylet(7, account).entry_type,
            LedgerEntryType::MPTokenIssuance
        );
        assert_eq!(
            mpt_issuance_keylet_from_id(key),
            Keylet::new(LedgerEntryType::MPTokenIssuance, key)
        );
        assert_eq!(
            mptoken_keylet(key, account).entry_type,
            LedgerEntryType::MPToken
        );
        assert_eq!(
            mptoken_keylet_from_id(key),
            Keylet::new(LedgerEntryType::MPToken, key)
        );
        assert_eq!(
            permissioned_domain_keylet(account, 9).entry_type,
            LedgerEntryType::PermissionedDomain
        );
        assert_eq!(
            permissioned_domain_keylet_from_id(key),
            Keylet::new(LedgerEntryType::PermissionedDomain, key)
        );

        assert_eq!(
            loan_keylet_from_key(key),
            Keylet::new(LedgerEntryType::Loan, key)
        );
        assert_eq!(
            loan_broker_keylet_from_key(key),
            Keylet::new(LedgerEntryType::LoanBroker, key)
        );
        assert_eq!(
            vault_keylet_from_key(key),
            Keylet::new(LedgerEntryType::Vault, key)
        );
        assert_eq!(
            directory_node_keylet(key),
            Keylet::new(LedgerEntryType::DirectoryNode, key)
        );
    }

    #[test]
    fn nft_page_keylets_preserve_the_current_mask_layout() {
        let owner = Uint160::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
            .expect("owner should parse");
        let token = Uint256::from_u64(7);
        let mask =
            Uint256::from_hex("0000000000000000000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF")
                .expect("mask should parse");

        let min = nft_page_min_keylet(owner);
        let max = nft_page_max_keylet(owner);
        let base = nft_page_keylet(min, token);

        assert_eq!(min.entry_type, LedgerEntryType::NFTokenPage);
        assert_eq!(max.entry_type, LedgerEntryType::NFTokenPage);
        assert_eq!(base.entry_type, LedgerEntryType::NFTokenPage);
        assert_eq!(min.key.data()[..20], owner.data()[..]);
        assert!(min.key.data()[20..].iter().all(|b| *b == 0));
        assert_eq!(max.key.data()[..20], owner.data()[..]);
        assert_eq!(
            base.key & mask,
            token & mask,
            "nft page key should preserve low 96 bits"
        );
        assert_eq!(
            base.key & !mask,
            min.key & !mask,
            "nft page key should preserve owner prefix"
        );
    }

    #[test]
    fn loan_key_matches_current_cpp_vector() {
        let loan_broker_id =
            Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
                .expect("expected loan broker id should parse");

        assert_eq!(
            loan_key(loan_broker_id, 7),
            Uint256::from_hex("B9CF90CA6D45957E6BB9A59666C328113077AA775B5B6516C8AFDDC507647E90")
                .expect("expected loan key should parse")
        );
        assert_eq!(
            loan_keylet(loan_broker_id, 7).entry_type,
            LedgerEntryType::Loan
        );
    }

    #[test]
    fn sequence_based_keylets_change_with_sequence() {
        let account = Uint160::from_hex("1111111111111111111111111111111111111111")
            .expect("expected account should parse");
        let other = Uint160::from_hex("2222222222222222222222222222222222222222")
            .expect("expected other account should parse");

        assert_ne!(offer_keylet(account, 1), offer_keylet(account, 2));
        assert_ne!(ticket_keylet(account, 1), ticket_keylet(account, 2));
        assert_ne!(check_keylet(account, 1), check_keylet(account, 2));
        assert_ne!(escrow_keylet(account, 1), escrow_keylet(account, 2));
        assert_ne!(vault_keylet(account, 1), vault_keylet(account, 2));
        assert_ne!(
            loan_broker_keylet(account, 1),
            loan_broker_keylet(account, 2)
        );
        assert_ne!(
            pay_channel_keylet(account, other, 1),
            pay_channel_keylet(account, other, 2)
        );
        assert_ne!(
            nft_offer_keylet_for_owner(account, 1),
            nft_offer_keylet_for_owner(account, 2)
        );
    }

    #[test]
    fn nft_offer_owner_sequence_keylet_matches_current_cpp_vector() {
        let owner = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected owner should parse");

        assert_eq!(
            nft_offer_keylet_for_owner(owner, 7).key,
            Uint256::from_hex("1BEA469F51623A9142E139B46807344E5C5B638ED5F36FD8E47E67CEE8910896")
                .expect("expected nft offer key should parse")
        );
    }

    #[test]
    fn ticket_keylet_from_seq_proxy_matches_ticket_overload() {
        let account = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected account should parse");

        assert_eq!(
            ticket_keylet_from_seq_proxy(account, SeqProxy::ticket(7)).key,
            Uint256::from_hex("38EF979A371455DF7B79A56CFB7F6840741BD83A26E07708C8964D9606909CA4")
                .expect("expected ticket key should parse")
        );
        assert_eq!(
            ticket_keylet_from_seq_proxy(account, SeqProxy::ticket(7)),
            ticket_keylet(account, 7)
        );
    }

    #[test]
    #[should_panic(expected = "ticket_keylet_from_seq_proxy requires a ticket SeqProxy")]
    fn ticket_keylet_from_seq_proxy_rejects_sequence_inputs_assert() {
        let _ = ticket_keylet_from_seq_proxy(Uint160::default(), SeqProxy::sequence(7));
    }

    #[test]
    fn signer_and_preauth_keylets_follow_current_input_shaping() {
        let owner = Uint160::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
            .expect("owner should parse");
        let peer = Uint160::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
            .expect("peer should parse");

        assert_eq!(
            signers_keylet(owner).entry_type,
            LedgerEntryType::SignerList
        );
        let raw = Uint256::from_u64(9);
        assert_eq!(
            offer_keylet_from_key(raw),
            Keylet::new(LedgerEntryType::Offer, raw)
        );
        assert_eq!(
            ticket_keylet_from_key(raw),
            Keylet::new(LedgerEntryType::Ticket, raw)
        );
        assert_eq!(
            check_keylet_from_key(raw),
            Keylet::new(LedgerEntryType::Check, raw)
        );
        assert_eq!(
            deposit_preauth_keylet_from_key(raw),
            Keylet::new(LedgerEntryType::DepositPreauth, raw)
        );
        assert_eq!(
            escrow_keylet_from_key(raw),
            Keylet::new(LedgerEntryType::Escrow, raw)
        );
        assert_eq!(
            pay_channel_keylet_from_key(raw),
            Keylet::new(LedgerEntryType::PayChannel, raw)
        );
        assert_eq!(signers_keylet(owner), signers_keylet_for_page(owner, 0));
        assert_ne!(
            signers_keylet_for_page(owner, 0),
            signers_keylet_for_page(owner, 1)
        );
        assert_eq!(
            deposit_preauth_keylet(owner, peer).entry_type,
            LedgerEntryType::DepositPreauth
        );
        assert_ne!(
            deposit_preauth_keylet(owner, peer),
            deposit_preauth_keylet(peer, owner)
        );
        assert_ne!(
            deposit_preauth_credentials_keylet(owner, &[Uint256::from_u64(1)]),
            deposit_preauth_credentials_keylet(owner, &[Uint256::from_u64(2)])
        );
        assert_ne!(
            deposit_preauth_credentials_keylet(owner, &[Uint256::from_u64(1)]),
            deposit_preauth_credentials_keylet(
                owner,
                &[Uint256::from_u64(1), Uint256::from_u64(2)]
            )
        );
    }

    #[test]
    fn directory_quality_page_and_next_match_current_cpp_vectors() {
        let owner = Uint160::from_hex("B5F762798A53D543A014CAF8B297CFF8F2F937E8")
            .expect("expected owner should parse");
        let root = owner_dir_keylet(owner);

        assert_eq!(
            quality_keylet(root, 0x1122_3344_5566_7788).key,
            Uint256::from_hex("D8120FC732737A2CF2E9968FDF3797A43B457F2A81AA06D21122334455667788")
                .expect("expected quality key should parse")
        );
        assert_eq!(page_keylet(root, 0), root);
        assert_eq!(
            page_keylet(root, 1).key,
            Uint256::from_hex("B001E91B2C4405A56F0BD0F6770A0B3230832C472667DFE9754933CA7F49A4F7")
                .expect("expected page key should parse")
        );
        assert_eq!(
            next_keylet(quality_keylet(root, 0x1122_3344_5566_7788)).key,
            Uint256::from_hex("D8120FC732737A2CF2E9968FDF3797A43B457F2A81AA06D31122334455667788")
                .expect("expected next quality key should parse")
        );
        assert_eq!(
            quality_from_key(
                Uint256::from_hex(
                    "D8120FC732737A2CF2E9968FDF3797A43B457F2A81AA06D21122334455667788"
                )
                .expect("expected quality key should parse")
            ),
            0x1122_3344_5566_7788
        );
    }

    #[test]
    fn nft_offer_directory_keylets_match_current_cpp_vectors() {
        let token_id =
            Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
                .expect("expected token id should parse");

        assert_eq!(
            nft_buy_offers_keylet(token_id).key,
            Uint256::from_hex("4BA5C0274A9FA4223ECAE038EF2307EA8F58CD6AF7CBE5E22BAF0FE7275E3B23")
                .expect("expected buy directory key should parse")
        );
        assert_eq!(
            nft_sell_offers_keylet(token_id).key,
            Uint256::from_hex("DC6B4198C90EBED069DF7D181F3D8937EFCBD25DA8A37AA372BF980902A0BBB6")
                .expect("expected sell directory key should parse")
        );
    }
}
