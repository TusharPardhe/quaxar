//! Transaction format registry ported from `TxFormats.*`.

use std::sync::OnceLock;

use crate::TxType;
use crate::known_formats::{KnownFormatItem, KnownFormats, KnownFormatsError};
use crate::sfield::get_field_by_symbol;
use crate::so_template::{SOEStyle, SOETxMPTIssue, SOElement, SOTemplate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxFormatMetadata {
    pub tag_name: &'static str,
    pub delegable: &'static str,
    pub amendment: &'static str,
    pub privileges: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxFormats {
    formats: KnownFormats<TxType, TxFormatMetadata>,
    common_fields: Vec<SOElement>,
}

#[derive(Debug)]
pub(crate) struct FormatFieldSpec {
    field_symbol: &'static str,
    style: SOEStyle,
    mpt: SOETxMPTIssue,
}

#[derive(Debug)]
pub(crate) struct TxFormatSpecInit {
    tag_name: &'static str,
    value: u16,
    name: &'static str,
    delegable: &'static str,
    amendment: &'static str,
    privileges: &'static str,
    field_specs: &'static [FormatFieldSpec],
}

impl TxFormats {
    fn build() -> Result<Self, KnownFormatsError> {
        let common_fields = build_elements(TX_COMMON_FIELD_SPECS)?;
        let mut formats = KnownFormats::new("TxFormats");

        for spec in TX_FORMAT_SPECS {
            let unique_fields = build_elements(spec.field_specs)?;
            let so_template = SOTemplate::new(unique_fields, common_fields.clone())?;
            formats.add(
                spec.name,
                TxType::from_u16(spec.value),
                so_template,
                TxFormatMetadata {
                    tag_name: spec.tag_name,
                    delegable: spec.delegable,
                    amendment: spec.amendment,
                    privileges: spec.privileges,
                },
            )?;
        }

        Ok(Self {
            formats,
            common_fields,
        })
    }

    pub fn get_instance() -> &'static Self {
        static INSTANCE: OnceLock<TxFormats> = OnceLock::new();
        INSTANCE.get_or_init(|| Self::build().expect("TxFormats registry should build"))
    }

    pub fn get_common_fields(&self) -> &[SOElement] {
        &self.common_fields
    }

    pub fn find_type_by_name(&self, name: &str) -> Result<TxType, KnownFormatsError> {
        self.formats.find_type_by_name(name)
    }

    pub fn find_by_type(
        &self,
        type_: TxType,
    ) -> Option<&KnownFormatItem<TxType, TxFormatMetadata>> {
        self.formats.find_by_type(type_)
    }

    pub fn find_by_name(&self, name: &str) -> Option<&KnownFormatItem<TxType, TxFormatMetadata>> {
        self.formats.find_by_name(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &KnownFormatItem<TxType, TxFormatMetadata>> {
        self.formats.iter()
    }
}

fn build_elements(specs: &[FormatFieldSpec]) -> Result<Vec<SOElement>, KnownFormatsError> {
    specs
        .iter()
        .map(|spec| {
            SOElement::new_with_mpt(get_field_by_symbol(spec.field_symbol), spec.style, spec.mpt)
                .map_err(KnownFormatsError::from)
        })
        .collect()
}

include!("generated_tx_format_specs.rs");
