//! Ledger object format registry ported from `LedgerFormats.*`.

use std::sync::OnceLock;

use crate::keylet::{LedgerEntryType, ledger_entry_type_from_code};
use crate::known_formats::{KnownFormatItem, KnownFormats, KnownFormatsError};
use crate::sfield::get_field_by_symbol;
use crate::so_template::{SOEStyle, SOElement, SOTemplate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerFormatMetadata {
    pub tag_name: &'static str,
    pub rpc_name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerFormats {
    formats: KnownFormats<LedgerEntryType, LedgerFormatMetadata>,
    common_fields: Vec<SOElement>,
}

#[derive(Debug)]
pub(crate) struct LedgerFormatFieldSpec {
    field_symbol: &'static str,
    style: SOEStyle,
}

#[derive(Debug)]
pub(crate) struct LedgerFormatSpecInit {
    tag_name: &'static str,
    value: u16,
    name: &'static str,
    rpc_name: &'static str,
    field_specs: &'static [LedgerFormatFieldSpec],
}

impl LedgerFormats {
    fn build() -> Result<Self, KnownFormatsError> {
        let common_fields = build_elements(LEDGER_COMMON_FIELD_SPECS)?;
        let mut formats = KnownFormats::new("LedgerFormats");

        for spec in LEDGER_FORMAT_SPECS {
            let unique_fields = build_elements(spec.field_specs)?;
            let so_template = SOTemplate::new(unique_fields, common_fields.clone())?;
            formats.add(
                spec.name,
                ledger_entry_type_from_code(spec.value)
                    .expect("generated ledger format must map to LedgerEntryType"),
                so_template,
                LedgerFormatMetadata {
                    tag_name: spec.tag_name,
                    rpc_name: spec.rpc_name,
                },
            )?;
        }

        Ok(Self {
            formats,
            common_fields,
        })
    }

    pub fn get_instance() -> &'static Self {
        static INSTANCE: OnceLock<LedgerFormats> = OnceLock::new();
        INSTANCE.get_or_init(|| Self::build().expect("LedgerFormats registry should build"))
    }

    pub fn get_common_fields(&self) -> &[SOElement] {
        &self.common_fields
    }

    pub fn find_type_by_name(&self, name: &str) -> Result<LedgerEntryType, KnownFormatsError> {
        self.formats.find_type_by_name(name)
    }

    pub fn find_by_type(
        &self,
        type_: LedgerEntryType,
    ) -> Option<&KnownFormatItem<LedgerEntryType, LedgerFormatMetadata>> {
        self.formats.find_by_type(type_)
    }

    pub fn find_by_name(
        &self,
        name: &str,
    ) -> Option<&KnownFormatItem<LedgerEntryType, LedgerFormatMetadata>> {
        self.formats.find_by_name(name)
    }

    pub fn iter(
        &self,
    ) -> impl Iterator<Item = &KnownFormatItem<LedgerEntryType, LedgerFormatMetadata>> {
        self.formats.iter()
    }
}

fn build_elements(specs: &[LedgerFormatFieldSpec]) -> Result<Vec<SOElement>, KnownFormatsError> {
    specs
        .iter()
        .map(|spec| {
            SOElement::new(get_field_by_symbol(spec.field_symbol), spec.style).map_err(Into::into)
        })
        .collect()
}

include!("generated_ledger_format_specs.rs");
