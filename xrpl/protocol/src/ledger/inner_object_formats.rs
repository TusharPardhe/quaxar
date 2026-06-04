//! `InnerObjectFormats` registry port from `xrpl/protocol/InnerObjectFormats.*`.

use std::sync::OnceLock;

use crate::{
    KnownFormatItem, KnownFormats, SField, SOEStyle, SOElement, SOTemplate, get_field_by_symbol,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerObjectFormats {
    inner: KnownFormats<i32>,
}

impl InnerObjectFormats {
    pub fn get_instance() -> &'static Self {
        static INSTANCE: OnceLock<InnerObjectFormats> = OnceLock::new();
        INSTANCE.get_or_init(Self::new)
    }

    fn new() -> Self {
        let mut inner = KnownFormats::new("InnerObjectFormats");

        add(
            &mut inner,
            "sfMemo",
            &[
                ("sfMemoType", SOEStyle::Optional),
                ("sfMemoData", SOEStyle::Optional),
                ("sfMemoFormat", SOEStyle::Optional),
            ],
        );
        add(
            &mut inner,
            "sfSignerEntry",
            &[
                ("sfAccount", SOEStyle::Required),
                ("sfSignerWeight", SOEStyle::Required),
                ("sfWalletLocator", SOEStyle::Optional),
            ],
        );
        add(
            &mut inner,
            "sfSigner",
            &[
                ("sfAccount", SOEStyle::Required),
                ("sfSigningPubKey", SOEStyle::Required),
                ("sfTxnSignature", SOEStyle::Required),
            ],
        );
        add(
            &mut inner,
            "sfMajority",
            &[
                ("sfAmendment", SOEStyle::Required),
                ("sfCloseTime", SOEStyle::Required),
            ],
        );
        add(
            &mut inner,
            "sfDisabledValidator",
            &[
                ("sfPublicKey", SOEStyle::Required),
                ("sfFirstLedgerSequence", SOEStyle::Required),
            ],
        );
        add(
            &mut inner,
            "sfNFToken",
            &[
                ("sfNFTokenID", SOEStyle::Required),
                ("sfURI", SOEStyle::Optional),
            ],
        );
        add(
            &mut inner,
            "sfVoteEntry",
            &[
                ("sfAccount", SOEStyle::Required),
                ("sfTradingFee", SOEStyle::Default),
                ("sfVoteWeight", SOEStyle::Required),
            ],
        );
        add(
            &mut inner,
            "sfAuctionSlot",
            &[
                ("sfAccount", SOEStyle::Required),
                ("sfExpiration", SOEStyle::Required),
                ("sfDiscountedFee", SOEStyle::Default),
                ("sfPrice", SOEStyle::Required),
                ("sfAuthAccounts", SOEStyle::Optional),
            ],
        );
        add(
            &mut inner,
            "sfXChainClaimAttestationCollectionElement",
            &[
                ("sfAttestationSignerAccount", SOEStyle::Required),
                ("sfPublicKey", SOEStyle::Required),
                ("sfSignature", SOEStyle::Required),
                ("sfAmount", SOEStyle::Required),
                ("sfAccount", SOEStyle::Required),
                ("sfAttestationRewardAccount", SOEStyle::Required),
                ("sfWasLockingChainSend", SOEStyle::Required),
                ("sfXChainClaimID", SOEStyle::Required),
                ("sfDestination", SOEStyle::Optional),
            ],
        );
        add(
            &mut inner,
            "sfXChainCreateAccountAttestationCollectionElement",
            &[
                ("sfAttestationSignerAccount", SOEStyle::Required),
                ("sfPublicKey", SOEStyle::Required),
                ("sfSignature", SOEStyle::Required),
                ("sfAmount", SOEStyle::Required),
                ("sfAccount", SOEStyle::Required),
                ("sfAttestationRewardAccount", SOEStyle::Required),
                ("sfWasLockingChainSend", SOEStyle::Required),
                ("sfXChainAccountCreateCount", SOEStyle::Required),
                ("sfDestination", SOEStyle::Required),
                ("sfSignatureReward", SOEStyle::Required),
            ],
        );
        add(
            &mut inner,
            "sfXChainClaimProofSig",
            &[
                ("sfAttestationSignerAccount", SOEStyle::Required),
                ("sfPublicKey", SOEStyle::Required),
                ("sfAmount", SOEStyle::Required),
                ("sfAttestationRewardAccount", SOEStyle::Required),
                ("sfWasLockingChainSend", SOEStyle::Required),
                ("sfDestination", SOEStyle::Optional),
            ],
        );
        add(
            &mut inner,
            "sfXChainCreateAccountProofSig",
            &[
                ("sfAttestationSignerAccount", SOEStyle::Required),
                ("sfPublicKey", SOEStyle::Required),
                ("sfAmount", SOEStyle::Required),
                ("sfSignatureReward", SOEStyle::Required),
                ("sfAttestationRewardAccount", SOEStyle::Required),
                ("sfWasLockingChainSend", SOEStyle::Required),
                ("sfDestination", SOEStyle::Required),
            ],
        );
        add(
            &mut inner,
            "sfAuthAccount",
            &[("sfAccount", SOEStyle::Required)],
        );
        add(
            &mut inner,
            "sfPriceData",
            &[
                ("sfBaseAsset", SOEStyle::Required),
                ("sfQuoteAsset", SOEStyle::Required),
                ("sfAssetPrice", SOEStyle::Optional),
                ("sfScale", SOEStyle::Default),
            ],
        );
        add(
            &mut inner,
            "sfCredential",
            &[
                ("sfIssuer", SOEStyle::Required),
                ("sfCredentialType", SOEStyle::Required),
            ],
        );
        add(
            &mut inner,
            "sfPermission",
            &[("sfPermissionValue", SOEStyle::Required)],
        );
        add(
            &mut inner,
            "sfRawTransaction",
            &[
                ("sfTransactionType", SOEStyle::Required),
                ("sfAccount", SOEStyle::Required),
                ("sfAmount", SOEStyle::Optional),
                ("sfDestination", SOEStyle::Optional),
                ("sfSequence", SOEStyle::Required),
                ("sfSigningPubKey", SOEStyle::Optional),
                ("sfTxnSignature", SOEStyle::Optional),
            ],
        );
        add(
            &mut inner,
            "sfBatchSigner",
            &[
                ("sfAccount", SOEStyle::Required),
                ("sfSigningPubKey", SOEStyle::Optional),
                ("sfTxnSignature", SOEStyle::Optional),
                ("sfSigners", SOEStyle::Optional),
            ],
        );
        add(
            &mut inner,
            "sfBook",
            &[
                ("sfBookDirectory", SOEStyle::Required),
                ("sfBookNode", SOEStyle::Required),
            ],
        );
        add(
            &mut inner,
            "sfCounterpartySignature",
            &[
                ("sfSigningPubKey", SOEStyle::Optional),
                ("sfTxnSignature", SOEStyle::Optional),
                ("sfSigners", SOEStyle::Optional),
            ],
        );

        Self { inner }
    }

    pub fn find_so_template_by_sfield(&self, sfield: &'static SField) -> Option<&SOTemplate> {
        self.inner
            .find_by_type(sfield.code())
            .map(|item| item.so_template())
    }

    pub fn iter(&self) -> impl Iterator<Item = &KnownFormatItem<i32>> {
        self.inner.iter()
    }
}

fn add(
    formats: &mut KnownFormats<i32>,
    sfield_symbol: &'static str,
    fields: &[(&'static str, SOEStyle)],
) {
    let sfield = get_field_by_symbol(sfield_symbol);
    let template = SOTemplate::new(
        fields
            .iter()
            .map(|(field_symbol, style)| SOElement::new(get_field_by_symbol(field_symbol), *style))
            .collect::<Result<Vec<_>, _>>()
            .expect("InnerObjectFormats field specs should remain valid"),
        Vec::new(),
    )
    .expect("InnerObjectFormats template specs should remain valid");
    formats
        .add(sfield.name(), sfield.code(), template, ())
        .expect("InnerObjectFormats entries should stay unique");
}
