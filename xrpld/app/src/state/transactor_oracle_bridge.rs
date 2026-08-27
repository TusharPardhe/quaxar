//! OracleSet transactor apply bridge.
//!
//! The transaction crate owns OracleSet's semantic merge and ordering rules.
//! This module is intentionally limited to lossless SLE/view adaptation.

use basics::math::base_uint::Uint160;
use ledger::{ApplyView, adjust_owner_count, dir_insert};
use protocol::{AccountID, STArray, STCurrency, STLedgerEntry, STObject, get_field_by_symbol};
use std::sync::Arc;
use tx::{
    OracleSetApplySink, OracleSetCreateMutation, OracleSetLoadedOracle, OracleSetSeriesEntry,
    OracleSetUpdateMutation, oracle_set_series_from_stobject,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn price_data_series(entries: Vec<OracleSetSeriesEntry>) -> STArray {
    let mut series = STArray::new(sf("sfPriceDataSeries"));
    series.reserve(entries.len());
    for entry in entries {
        let mut price_data = STObject::make_inner_object(sf("sfPriceData"));
        price_data.set_field_currency(
            sf("sfBaseAsset"),
            STCurrency::new_with_currency(sf("sfBaseAsset"), entry.pair.base_asset),
        );
        price_data.set_field_currency(
            sf("sfQuoteAsset"),
            STCurrency::new_with_currency(sf("sfQuoteAsset"), entry.pair.quote_asset),
        );
        if let Some(asset_price) = entry.asset_price {
            price_data.set_field_u64(sf("sfAssetPrice"), asset_price);
        }
        if let Some(scale) = entry.scale {
            price_data.set_field_u8(sf("sfScale"), scale as u8);
        }
        series.push_back(price_data);
    }
    series
}

pub struct ViewBackedOracleSetSink<'a, V> {
    pub view: &'a mut V,
    pub account: AccountID,
    pub oracle_document_id: u32,
    pub failure: Option<protocol::Ter>,
}

impl<V: ApplyView> ViewBackedOracleSetSink<'_, V> {
    fn oracle_keylet(&self) -> protocol::Keylet {
        protocol::oracle_keylet(
            Uint160::from_void(self.account.data()),
            self.oracle_document_id,
        )
    }
}

impl<V: ApplyView> OracleSetApplySink for ViewBackedOracleSetSink<'_, V> {
    fn existing_oracle(&mut self) -> Result<Option<OracleSetLoadedOracle>, protocol::Ter> {
        self.view
            .peek(self.oracle_keylet())
            .map_err(|_| protocol::Ter::TEF_BAD_LEDGER)
            .map(|oracle| {
                oracle.map(|oracle| OracleSetLoadedOracle {
                    has_oracle_document_id: oracle.is_field_present(sf("sfOracleDocumentID")),
                    price_data_series: oracle_set_series_from_stobject(&oracle),
                })
            })
    }

    fn fix_include_keylet_fields_enabled(&mut self) -> bool {
        self.view
            .rules()
            .enabled(&protocol::feature_id("fixIncludeKeyletFields"))
    }

    fn fix_price_oracle_order_enabled(&mut self) -> bool {
        self.view
            .rules()
            .enabled(&protocol::feature_id("fixPriceOracleOrder"))
    }

    fn adjust_owner_count(&mut self, delta: i8) -> bool {
        let account = match self.view.peek(protocol::account_keylet(Uint160::from_void(
            self.account.data(),
        ))) {
            Ok(Some(account)) => account,
            Ok(None) => {
                self.failure = Some(protocol::Ter::TEF_INTERNAL);
                return false;
            }
            Err(_) => {
                self.failure = Some(protocol::Ter::TEF_BAD_LEDGER);
                return false;
            }
        };
        if adjust_owner_count(self.view, &account, i32::from(delta)).is_err() {
            self.failure = Some(protocol::Ter::TEF_BAD_LEDGER);
            return false;
        }
        true
    }

    fn update_existing_oracle(&mut self, mutation: OracleSetUpdateMutation) -> bool {
        let oracle = match self.view.peek(self.oracle_keylet()) {
            Ok(Some(oracle)) => oracle,
            Ok(None) => {
                self.failure = Some(protocol::Ter::TEF_INTERNAL);
                return false;
            }
            Err(_) => {
                self.failure = Some(protocol::Ter::TEF_BAD_LEDGER);
                return false;
            }
        };
        let mut object = oracle.clone_as_object();
        object.set_field_array(
            sf("sfPriceDataSeries"),
            price_data_series(mutation.updated_series),
        );
        if let Some(uri) = mutation.uri {
            object.set_field_vl(sf("sfURI"), &uri);
        }
        object.set_field_u32(
            sf("sfLastUpdateTime"),
            mutation.last_update_time_secs as u32,
        );
        if mutation.set_oracle_document_id {
            object.set_field_u32(sf("sfOracleDocumentID"), mutation.oracle_document_id);
        }
        let updated = self
            .view
            .update(Arc::new(STLedgerEntry::from_stobject(
                object,
                *oracle.key(),
            )))
            .is_ok();
        if !updated {
            self.failure = Some(protocol::Ter::TEF_BAD_LEDGER);
        }
        updated
    }

    fn insert_owner_dir(&mut self) -> Result<Option<u64>, protocol::Ter> {
        let oracle_keylet = self.oracle_keylet();
        dir_insert(
            self.view,
            &protocol::owner_dir_keylet(Uint160::from_void(self.account.data())),
            oracle_keylet.key,
            &ledger::describe_owner_dir(self.account),
        )
        .map_err(|_| protocol::Ter::TEF_BAD_LEDGER)
    }

    fn create_oracle(&mut self, mutation: OracleSetCreateMutation) -> bool {
        let mut oracle = STLedgerEntry::new(self.oracle_keylet());
        oracle.set_account_id(sf("sfOwner"), self.account);
        if mutation.include_oracle_document_id {
            oracle.set_field_u32(sf("sfOracleDocumentID"), mutation.oracle_document_id);
        }
        oracle.set_field_vl(sf("sfProvider"), &mutation.provider);
        if let Some(uri) = mutation.uri {
            oracle.set_field_vl(sf("sfURI"), &uri);
        }
        oracle.set_field_array(
            sf("sfPriceDataSeries"),
            price_data_series(mutation.price_data_series),
        );
        oracle.set_field_vl(sf("sfAssetClass"), &mutation.asset_class);
        oracle.set_field_u32(
            sf("sfLastUpdateTime"),
            mutation.last_update_time_secs as u32,
        );
        oracle.set_field_u64(sf("sfOwnerNode"), mutation.owner_node);
        let inserted = self.view.insert(Arc::new(oracle)).is_ok();
        if !inserted {
            self.failure = Some(protocol::Ter::TEF_BAD_LEDGER);
        }
        inserted
    }
}
