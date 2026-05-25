//! Runtime asset-association seam from `xrpl/protocol/STTakesAsset.*`.

use crate::{Asset, SField, SOEStyle, STLedgerEntry, STNumber, StBase};

pub trait StTakesAsset: StBase {
    fn associate_asset(&mut self, asset: Asset);
}

impl StTakesAsset for STNumber {
    fn associate_asset(&mut self, asset: Asset) {
        STNumber::associate_asset(self, asset);
    }
}

pub fn associate_asset(sle: &mut STLedgerEntry, asset: Asset) {
    for index in 0..sle.get_count() {
        let field = sle.get_index(index).fname();
        if !field.should_meta(SField::S_MD_NEEDS_ASSET) {
            continue;
        }

        if sle.get_index(index).stype() == crate::SerializedTypeId::NotPresent {
            continue;
        }

        assert_eq!(
            sle.get_index(index).stype(),
            crate::SerializedTypeId::Number,
            "Current S_MD_NEEDS_ASSET fields should remain STNumber-backed",
        );

        if let Some(number) = sle
            .get_index_mut(index)
            .as_any_mut()
            .downcast_mut::<STNumber>()
        {
            number.associate_asset(asset);
        } else {
            panic!(
                "Field '{}' needs an STTakesAsset implementation",
                field.name()
            );
        }

        if matches!(sle.get_style(field), Some(SOEStyle::Default))
            && sle.get_index(index).is_default()
        {
            sle.make_field_absent(field);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{SerializedTypeId, all_sfields};

    use super::*;

    #[test]
    fn all_current_needs_asset_fields_are_stnumber_backed() {
        let fields = all_sfields()
            .iter()
            .filter(|field| field.should_meta(SField::S_MD_NEEDS_ASSET))
            .collect::<Vec<_>>();

        assert!(!fields.is_empty());
        assert!(
            fields
                .iter()
                .all(|field| field.field_type() == SerializedTypeId::Number)
        );
    }
}
