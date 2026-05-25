//! `STCurrency` port from `xrpl/protocol/STCurrency.*`.

use crate::{
    Currency, JsonOptions, JsonValue, SField, SerialIter, SerializedTypeId, Serializer, StBase,
    StBaseCore, currency_to_string, downcast_stbase_ref, is_xrp_currency,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct STCurrency {
    core: StBaseCore,
    currency: Currency,
}

impl STCurrency {
    pub fn with_field(field: &'static SField) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            currency: Currency::zero(),
        }
    }

    pub fn from_serial_iter(sit: &mut SerialIter<'_>, field: &'static SField) -> Self {
        Self::new_with_currency(
            field,
            Currency::from_slice(sit.get160().data()).expect("currency width should match"),
        )
    }

    pub fn new_with_currency(field: &'static SField, currency: Currency) -> Self {
        Self {
            core: StBaseCore::with_field(field),
            currency,
        }
    }

    pub fn currency(&self) -> Currency {
        self.currency
    }

    pub fn set_currency(&mut self, currency: Currency) {
        self.currency = currency;
    }
}

impl StBase for STCurrency {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn core(&self) -> &StBaseCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut StBaseCore {
        &mut self.core
    }

    fn stype(&self) -> SerializedTypeId {
        SerializedTypeId::Currency
    }

    fn text(&self) -> String {
        currency_to_string(self.currency)
    }

    fn json(&self, _options: JsonOptions) -> JsonValue {
        JsonValue::String(self.text())
    }

    fn add(&self, serializer: &mut Serializer) {
        serializer.add_bit_string(self.currency);
    }

    fn is_equivalent(&self, other: &dyn StBase) -> bool {
        downcast_stbase_ref::<Self>(other).currency == self.currency
    }

    fn is_default(&self) -> bool {
        is_xrp_currency(self.currency)
    }
}
