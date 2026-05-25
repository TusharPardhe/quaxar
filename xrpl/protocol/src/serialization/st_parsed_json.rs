//! Parsed-JSON wrapper from `xrpl/protocol/STParsedJSON.*`.

use basics::{
    base_uint::{Uint128, Uint160, Uint192, Uint256},
    buffer::Buffer,
    string_utilities::str_unhex,
};

use crate::{
    JsonValue, LedgerFormats, Permission, STAccount, STAmount, STArray, STBlob, STCurrency,
    STInt32, STIssue, STNumber, STObject, STUInt8, STUInt16, STUInt32, STUInt64, STUInt128,
    STUInt160, STUInt192, STUInt256, STVector256, STXChainBridge, SerializedTypeId, TxFormats,
    TxType, asset_from_json, currency_from_string, get_field_by_name, get_field_by_symbol,
    make_param_error, normalized_parts_from_json_input, normalized_parts_from_string,
    parse_base58_account_id, st_issue_from_json, st_path_set_from_json,
};

#[derive(Debug, Clone)]
pub struct STParsedJSONObject {
    pub object: Option<STObject>,
    pub error: JsonValue,
}

const MAX_PARSED_JSON_DEPTH: usize = 64;

impl STParsedJSONObject {
    pub fn new(name: &str, json: &JsonValue) -> Self {
        match parse_object(get_field_by_symbol("sfGeneric"), name, json, 0) {
            Ok(object) => Self {
                object: Some(object),
                error: JsonValue::Null,
            },
            Err(message) => Self {
                object: None,
                error: make_param_error(&message),
            },
        }
    }
}

fn parse_object(
    field: &'static crate::SField,
    name: &str,
    json: &JsonValue,
    depth: usize,
) -> Result<STObject, String> {
    if depth > MAX_PARSED_JSON_DEPTH {
        return Err(format!("Field '{name}' exceeds the maximum nesting depth."));
    }

    let JsonValue::Object(entries) = json else {
        return Err(format!("Field '{name}' is not a JSON object."));
    };

    let mut object = if field == get_field_by_symbol("sfGeneric") {
        STObject::new(field)
    } else {
        STObject::make_inner_object(field)
    };

    for (json_field_name, value) in entries {
        let sfield = get_field_by_name(json_field_name);
        if sfield.is_invalid() {
            return Err(format!("Field '{name}.{json_field_name}' is unknown."));
        }
        set_field_from_json(
            &mut object,
            sfield,
            &format!("{name}.{json_field_name}"),
            value,
            depth + 1,
        )?;
    }

    object.apply_template_from_sfield(field);

    if object.is_field_present(get_field_by_symbol("sfTransactionType")) {
        let tx_type =
            TxType::from_u16(object.get_field_u16(get_field_by_symbol("sfTransactionType")));
        let Some(item) = TxFormats::get_instance().find_by_type(tx_type) else {
            return Err("Field 'TransactionType' is unknown.".to_owned());
        };
        object.apply_template(item.so_template());
    }

    if object.is_field_present(get_field_by_symbol("sfLedgerEntryType")) {
        let code = object.get_field_u16(get_field_by_symbol("sfLedgerEntryType"));
        let Some(entry_type) = crate::keylet::ledger_entry_type_from_code(code) else {
            return Err("Field 'LedgerEntryType' is unknown.".to_owned());
        };
        let Some(item) = LedgerFormats::get_instance().find_by_type(entry_type) else {
            return Err("Field 'LedgerEntryType' is unknown.".to_owned());
        };
        object.apply_template(item.so_template());
    }

    Ok(object)
}

fn set_field_from_json(
    object: &mut STObject,
    field: &'static crate::SField,
    name: &str,
    value: &JsonValue,
    depth: usize,
) -> Result<(), String> {
    match field.field_type() {
        SerializedTypeId::UInt8 => object.set_stbase(parse_u8_field(field, value)?),
        SerializedTypeId::UInt16 => object.set_stbase(parse_u16_field(field, value)?),
        SerializedTypeId::UInt32 => object.set_stbase(parse_u32_field(field, value)?),
        SerializedTypeId::UInt64 => object.set_stbase(parse_u64_field(field, value)?),
        SerializedTypeId::Int32 => object.set_stbase(parse_i32_field(field, value)?),
        SerializedTypeId::UInt128 => object.set_stbase(parse_uint128_field(field, value)?),
        SerializedTypeId::UInt160 => object.set_stbase(parse_uint160_field(field, value)?),
        SerializedTypeId::UInt192 => object.set_stbase(parse_uint192_field(field, value)?),
        SerializedTypeId::UInt256 => object.set_stbase(parse_uint256_field(field, value)?),
        SerializedTypeId::Account => object.set_stbase(parse_account_field(field, value)?),
        SerializedTypeId::VariableLength => object.set_stbase(parse_blob_field(field, value)?),
        SerializedTypeId::Amount => {
            object.set_field_amount(field, parse_amount_field(field, value)?)
        }
        SerializedTypeId::Number => {
            object.set_field_number(field, parse_number_field(field, value)?)
        }
        SerializedTypeId::Currency => {
            object.set_field_currency(field, parse_currency_field(field, value)?)
        }
        SerializedTypeId::Issue => object.set_field_issue(field, parse_issue_field(field, value)?),
        SerializedTypeId::Array => {
            object.set_field_array(field, parse_array_field(field, name, value, depth)?)
        }
        SerializedTypeId::Object => {
            object.set_field_object(field, parse_object(field, name, value, depth)?)
        }
        SerializedTypeId::PathSet => {
            object.set_field_path_set(field, st_path_set_from_json(field, value)?)
        }
        SerializedTypeId::Vector256 => {
            object.set_field_v256(field, parse_vector256_field(field, value)?)
        }
        SerializedTypeId::XChainBridge => object.set_field_xchain_bridge(
            field,
            STXChainBridge::from_json_value(field, value).map_err(|_| invalid_data(name))?,
        ),
        other => return Err(format!("Field '{name}' has unsupported type {other:?}.")),
    }

    Ok(())
}

fn parse_u8_field(field: &'static crate::SField, value: &JsonValue) -> Result<STUInt8, String> {
    let parsed = match value {
        JsonValue::Bool(value) => u8::from(*value),
        JsonValue::Unsigned(value) => u8::try_from(*value).map_err(|_| field_invalid(field))?,
        JsonValue::Signed(value) if *value >= 0 => {
            u8::try_from(*value).map_err(|_| field_invalid(field))?
        }
        JsonValue::String(value) => value.parse::<u8>().map_err(|_| field_invalid(field))?,
        _ => return Err(field_invalid(field)),
    };
    Ok(STUInt8::with_field(field, parsed))
}

fn parse_u16_field(field: &'static crate::SField, value: &JsonValue) -> Result<STUInt16, String> {
    let parsed = if field == get_field_by_symbol("sfTransactionType") {
        parse_transaction_type(value)?.to_u16()
    } else if field == get_field_by_symbol("sfLedgerEntryType") {
        parse_ledger_entry_type(value)? as u16
    } else {
        match value {
            JsonValue::Unsigned(value) => {
                u16::try_from(*value).map_err(|_| field_invalid(field))?
            }
            JsonValue::Signed(value) if *value >= 0 => {
                u16::try_from(*value).map_err(|_| field_invalid(field))?
            }
            JsonValue::String(value) => value.parse::<u16>().map_err(|_| field_invalid(field))?,
            _ => return Err(field_invalid(field)),
        }
    };
    Ok(STUInt16::with_field(field, parsed))
}

fn parse_u32_field(field: &'static crate::SField, value: &JsonValue) -> Result<STUInt32, String> {
    let parsed = if field == get_field_by_symbol("sfPermissionValue") {
        parse_permission_value(value)?
    } else {
        match value {
            JsonValue::Unsigned(value) => {
                u32::try_from(*value).map_err(|_| field_invalid(field))?
            }
            JsonValue::Signed(value) if *value >= 0 => {
                u32::try_from(*value).map_err(|_| field_invalid(field))?
            }
            JsonValue::String(value) => value.parse::<u32>().map_err(|_| field_invalid(field))?,
            _ => return Err(field_invalid(field)),
        }
    };
    Ok(STUInt32::with_field(field, parsed))
}

fn parse_u64_field(field: &'static crate::SField, value: &JsonValue) -> Result<STUInt64, String> {
    let parsed = match value {
        JsonValue::Unsigned(value) => *value,
        JsonValue::Signed(value) if *value >= 0 => *value as u64,
        JsonValue::String(value) => {
            let radix = if field.should_meta(crate::SField::S_MD_BASE_TEN) {
                10
            } else {
                16
            };
            u64::from_str_radix(value, radix).map_err(|_| field_invalid(field))?
        }
        _ => return Err(field_invalid(field)),
    };
    Ok(STUInt64::with_field(field, parsed))
}

fn parse_i32_field(field: &'static crate::SField, value: &JsonValue) -> Result<STInt32, String> {
    let parsed = match value {
        JsonValue::Signed(value) => i32::try_from(*value).map_err(|_| field_invalid(field))?,
        JsonValue::Unsigned(value) => i32::try_from(*value).map_err(|_| field_invalid(field))?,
        JsonValue::String(value) => value.parse::<i32>().map_err(|_| field_invalid(field))?,
        _ => return Err(field_invalid(field)),
    };
    Ok(STInt32::with_field(field, parsed))
}

fn parse_uint128_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STUInt128, String> {
    let JsonValue::String(text) = value else {
        return Err(field_invalid(field));
    };
    let parsed = Uint128::from_slice(&parse_fixed_width_hex(text, Uint128::BYTES)?)
        .ok_or_else(|| field_invalid(field))?;
    Ok(STUInt128::with_field(field, parsed))
}

fn parse_uint160_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STUInt160, String> {
    let JsonValue::String(text) = value else {
        return Err(field_invalid(field));
    };
    let parsed = Uint160::from_slice(&parse_fixed_width_hex(text, Uint160::BYTES)?)
        .ok_or_else(|| field_invalid(field))?;
    Ok(STUInt160::with_field(field, parsed))
}

fn parse_uint192_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STUInt192, String> {
    let JsonValue::String(text) = value else {
        return Err(field_invalid(field));
    };
    let parsed = Uint192::from_slice(&parse_fixed_width_hex(text, Uint192::BYTES)?)
        .ok_or_else(|| field_invalid(field))?;
    Ok(STUInt192::with_field(field, parsed))
}

fn parse_uint256_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STUInt256, String> {
    let JsonValue::String(text) = value else {
        return Err(field_invalid(field));
    };
    let parsed = Uint256::from_slice(&parse_fixed_width_hex(text, Uint256::BYTES)?)
        .ok_or_else(|| field_invalid(field))?;
    Ok(STUInt256::with_field(field, parsed))
}

fn parse_fixed_width_hex(text: &str, expected_bytes: usize) -> Result<Vec<u8>, String> {
    if text.is_empty() {
        return Ok(vec![0; expected_bytes]);
    }

    let Some(bytes) = str_unhex(text) else {
        return Err("invalid hex".to_owned());
    };
    if bytes.len() != expected_bytes {
        return Err("wrong width".to_owned());
    }
    Ok(bytes)
}

fn parse_account_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STAccount, String> {
    let JsonValue::String(text) = value else {
        return Err(field_invalid(field));
    };
    let account = parse_base58_account_id(text)
        .or_else(|| crate::AccountID::from_hex(text).ok())
        .ok_or_else(|| field_invalid(field))?;
    Ok(STAccount::from_value(field, account))
}

fn parse_blob_field(field: &'static crate::SField, value: &JsonValue) -> Result<STBlob, String> {
    let JsonValue::String(text) = value else {
        return Err(field_invalid(field));
    };
    let Some(bytes) = str_unhex(text) else {
        return Err(field_invalid(field));
    };
    Ok(STBlob::from_buffer(field, Buffer::from_bytes(&bytes)))
}

fn parse_amount_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STAmount, String> {
    match value {
        JsonValue::String(text) => {
            let runtime = normalized_parts_from_string(text).map_err(|_| field_invalid(field))?;
            let xrp = crate::XRPAmount::from_number(runtime).map_err(|_| field_invalid(field))?;
            Ok(STAmount::from_xrp_amount(xrp))
        }
        JsonValue::Object(object) => {
            let Some(JsonValue::String(text)) = object.get("value") else {
                return Err(field_invalid(field));
            };
            let asset = asset_from_json(value).map_err(|_| field_invalid(field))?;
            let runtime = normalized_parts_from_string(text).map_err(|_| field_invalid(field))?;
            match asset {
                crate::Asset::Issue(issue) if issue.native() => {
                    let xrp =
                        crate::XRPAmount::from_number(runtime).map_err(|_| field_invalid(field))?;
                    Ok(STAmount::from_xrp_amount(xrp))
                }
                crate::Asset::Issue(issue) => {
                    let amount =
                        crate::IOUAmount::from_number(runtime).map_err(|_| field_invalid(field))?;
                    Ok(STAmount::from_iou_amount(field, amount, issue))
                }
                crate::Asset::MPTIssue(issue) => {
                    let amount =
                        crate::MPTAmount::from_number(runtime).map_err(|_| field_invalid(field))?;
                    Ok(STAmount::from_mpt_amount(field, amount, issue))
                }
            }
        }
        _ => Err(field_invalid(field)),
    }
}

fn parse_number_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STNumber, String> {
    let runtime = match value {
        JsonValue::Signed(value) => {
            normalized_parts_from_json_input(crate::NumberJsonInput::Int(*value))
        }
        JsonValue::Unsigned(value) => {
            normalized_parts_from_json_input(crate::NumberJsonInput::UInt(*value))
        }
        JsonValue::String(value) => {
            normalized_parts_from_json_input(crate::NumberJsonInput::String(value))
        }
        _ => normalized_parts_from_json_input(crate::NumberJsonInput::Other),
    }
    .map_err(|_| field_invalid(field))?;

    Ok(STNumber::with_field(field, runtime))
}

fn parse_currency_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STCurrency, String> {
    let JsonValue::String(text) = value else {
        return Err(field_invalid(field));
    };
    Ok(STCurrency::new_with_currency(
        field,
        currency_from_string(text),
    ))
}

fn parse_issue_field(field: &'static crate::SField, value: &JsonValue) -> Result<STIssue, String> {
    st_issue_from_json(field, value).map_err(|_| field_invalid(field))
}

fn parse_array_field(
    field: &'static crate::SField,
    name: &str,
    value: &JsonValue,
    depth: usize,
) -> Result<STArray, String> {
    if depth > MAX_PARSED_JSON_DEPTH {
        return Err(format!("Field '{name}' exceeds the maximum nesting depth."));
    }

    let JsonValue::Array(entries) = value else {
        return Err(format!("Field '{name}' is not a JSON array."));
    };

    let mut array = STArray::new(field);
    for (index, entry) in entries.iter().enumerate() {
        let JsonValue::Object(singleton) = entry else {
            return Err(format!("Item '{name}' at index {index} is not an object."));
        };
        if singleton.len() != 1 {
            return Err(format!(
                "Field '{name}[{index}]' must be an object with a single key/object value."
            ));
        }
        let (inner_name, inner_value) = singleton.iter().next().expect("singleton object");
        let inner_field = get_field_by_name(inner_name);
        if inner_field.is_invalid() {
            return Err(format!("Field '{name}[{index}].{inner_name}' is unknown."));
        }
        array.push_back(parse_object(
            inner_field,
            &format!("{name}[{index}]"),
            inner_value,
            depth + 1,
        )?);
    }

    Ok(array)
}

fn parse_vector256_field(
    field: &'static crate::SField,
    value: &JsonValue,
) -> Result<STVector256, String> {
    let JsonValue::Array(entries) = value else {
        return Err(field_invalid(field));
    };
    let mut vector = STVector256::with_field(field);
    for entry in entries {
        let JsonValue::String(text) = entry else {
            return Err(field_invalid(field));
        };
        vector.push_back(Uint256::from_hex(text).map_err(|_| field_invalid(field))?);
    }
    Ok(vector)
}

fn parse_transaction_type(value: &JsonValue) -> Result<TxType, String> {
    match value {
        JsonValue::String(value) => TxFormats::get_instance()
            .find_type_by_name(value)
            .or_else(|_| {
                TxType::from_tag_name(value).ok_or_else(|| {
                    crate::KnownFormatsError::UnknownFormatName {
                        registry: "TxFormats",
                        name: value.clone(),
                    }
                })
            })
            .map_err(|_| "invalid transaction type".to_owned()),
        JsonValue::Unsigned(value) => Ok(TxType::from_u16(*value as u16)),
        JsonValue::Signed(value) if *value >= 0 => Ok(TxType::from_u16(*value as u16)),
        _ => Err("invalid transaction type".to_owned()),
    }
}

fn parse_ledger_entry_type(value: &JsonValue) -> Result<crate::LedgerEntryType, String> {
    match value {
        JsonValue::String(value) => LedgerFormats::get_instance()
            .find_by_name(value)
            .map(|item| item.format_type())
            .ok_or_else(|| "invalid ledger entry type".to_owned()),
        JsonValue::Unsigned(value) => crate::keylet::ledger_entry_type_from_code(*value as u16)
            .ok_or_else(|| "invalid ledger entry type".to_owned()),
        JsonValue::Signed(value) if *value >= 0 => {
            crate::keylet::ledger_entry_type_from_code(*value as u16)
                .ok_or_else(|| "invalid ledger entry type".to_owned())
        }
        _ => Err("invalid ledger entry type".to_owned()),
    }
}

fn parse_permission_value(value: &JsonValue) -> Result<u32, String> {
    match value {
        JsonValue::String(value) => Permission::get_instance()
            .get_permission_value(value)
            .ok_or_else(|| "invalid permission value".to_owned()),
        JsonValue::Unsigned(value) => {
            u32::try_from(*value).map_err(|_| "invalid permission value".to_owned())
        }
        JsonValue::Signed(value) if *value >= 0 => {
            u32::try_from(*value).map_err(|_| "invalid permission value".to_owned())
        }
        _ => Err("invalid permission value".to_owned()),
    }
}

fn field_invalid(field: &'static crate::SField) -> String {
    format!("Field '{}' has invalid data.", field.name())
}

fn invalid_data(name: &str) -> String {
    format!("Field '{name}' has invalid data.")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use basics::base_uint::{Uint128, Uint160, Uint192, Uint256};

    use super::STParsedJSONObject;
    use crate::{JsonValue, Permission, StBase, get_field_by_symbol};

    #[test]
    fn parsed_json_handles_permission_names_and_transaction_type_strings() {
        let json = JsonValue::Object(BTreeMap::from([
            (
                "TransactionType".to_owned(),
                JsonValue::String("DelegateSet".to_owned()),
            ),
            (
                "Account".to_owned(),
                JsonValue::String("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh".to_owned()),
            ),
            (
                "Authorize".to_owned(),
                JsonValue::String("rPT1Sjq2YGrBMTttX4GZHjKu9dyfzbpAYe".to_owned()),
            ),
            ("SigningPubKey".to_owned(), JsonValue::String("".to_owned())),
            ("Fee".to_owned(), JsonValue::String("10".to_owned())),
            ("Sequence".to_owned(), JsonValue::Unsigned(1)),
            (
                "Permissions".to_owned(),
                JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([(
                    "Permission".to_owned(),
                    JsonValue::Object(BTreeMap::from([(
                        "PermissionValue".to_owned(),
                        JsonValue::String("PaymentMint".to_owned()),
                    )])),
                )]))]),
            ),
        ]));

        let parsed = STParsedJSONObject::new("tx_json", &json);
        let object = parsed.object.expect("object should parse");
        let permissions = object.get_field_array(get_field_by_symbol("sfPermissions"));
        let permission = permissions.get(0).expect("permission element");

        assert_eq!(
            permission.get_field_u32(get_field_by_symbol("sfPermissionValue")),
            Permission::get_instance()
                .get_granular_value("PaymentMint")
                .expect("granular permission")
        );
    }

    #[test]
    fn parsed_json_uint64_strings_follow_hex_and_base_ten_field_rules() {
        let hex_json = JsonValue::Object(BTreeMap::from([(
            "IndexNext".to_owned(),
            JsonValue::String("ffffffffffffffff".to_owned()),
        )]));
        let hex = STParsedJSONObject::new("test", &hex_json)
            .object
            .expect("hex uint64 should parse");
        assert_eq!(
            hex.get_field_u64(get_field_by_symbol("sfIndexNext")),
            u64::MAX
        );

        let base_ten_json = JsonValue::Object(BTreeMap::from([(
            "MaximumAmount".to_owned(),
            JsonValue::String("18446744073709551615".to_owned()),
        )]));
        let base_ten = STParsedJSONObject::new("test", &base_ten_json)
            .object
            .expect("base ten uint64 should parse");
        assert_eq!(
            base_ten.get_field_u64(get_field_by_symbol("sfMaximumAmount")),
            u64::MAX
        );
    }

    #[test]
    fn parsed_json_fixed_width_uints_accept_empty_string_as_zero() {
        let json = JsonValue::Object(BTreeMap::from([
            ("EmailHash".to_owned(), JsonValue::String(String::new())),
            (
                "TakerPaysCurrency".to_owned(),
                JsonValue::String(String::new()),
            ),
            (
                "MPTokenIssuanceID".to_owned(),
                JsonValue::String(String::new()),
            ),
            ("LedgerHash".to_owned(), JsonValue::String(String::new())),
        ]));

        let object = STParsedJSONObject::new("test", &json)
            .object
            .expect("empty fixed-width hex fields should parse");
        assert_eq!(
            object.get_field_h128(get_field_by_symbol("sfEmailHash")),
            Uint128::zero()
        );
        assert_eq!(
            object.get_field_h160(get_field_by_symbol("sfTakerPaysCurrency")),
            Uint160::zero()
        );
        assert_eq!(
            object.get_field_h192(get_field_by_symbol("sfMPTokenIssuanceID")),
            Uint192::zero()
        );
        assert_eq!(
            object.get_field_h256(get_field_by_symbol("sfLedgerHash")),
            Uint256::zero()
        );
    }

    #[test]
    fn parsed_json_object_and_array_rules_match_cpp_depth_and_singleton_behavior() {
        let object_json = JsonValue::Object(BTreeMap::from([(
            "TransactionMetaData".to_owned(),
            JsonValue::Object(BTreeMap::from([(
                "TransactionResult".to_owned(),
                JsonValue::Unsigned(1),
            )])),
        )]));
        let parsed_object = STParsedJSONObject::new("test", &object_json)
            .object
            .expect("object field should parse");
        assert_eq!(
            parsed_object
                .get_field_object(get_field_by_symbol("sfTransactionMetaData"))
                .get_field_u8(get_field_by_symbol("sfTransactionResult")),
            1
        );

        let array_json = JsonValue::Object(BTreeMap::from([(
            "SignerEntries".to_owned(),
            JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([(
                "TransactionMetaData".to_owned(),
                JsonValue::Object(BTreeMap::from([(
                    "TransactionResult".to_owned(),
                    JsonValue::Unsigned(2),
                )])),
            )]))]),
        )]));
        let parsed_array = STParsedJSONObject::new("test", &array_json)
            .object
            .expect("array field should parse");
        let entries = parsed_array.get_field_array(get_field_by_symbol("sfSignerEntries"));
        let first = entries.get(0).expect("single array element");
        assert_eq!(first.fname(), get_field_by_symbol("sfTransactionMetaData"));
        assert_eq!(
            first.get_field_u8(get_field_by_symbol("sfTransactionResult")),
            2
        );

        let invalid_array_json = JsonValue::Object(BTreeMap::from([(
            "SignerEntries".to_owned(),
            JsonValue::Array(vec![JsonValue::Object(BTreeMap::from([
                ("TransactionResult".to_owned(), JsonValue::Unsigned(2)),
                ("NetworkID".to_owned(), JsonValue::Unsigned(3)),
            ]))]),
        )]));
        assert!(
            STParsedJSONObject::new("test", &invalid_array_json)
                .object
                .is_none()
        );
    }

    #[test]
    fn parsed_json_rejects_object_depth_beyond_cpp_limit() {
        let mut nested = JsonValue::Object(BTreeMap::from([(
            "TransactionResult".to_owned(),
            JsonValue::Unsigned(1),
        )]));
        for _ in 0..64 {
            nested =
                JsonValue::Object(BTreeMap::from([("TransactionMetaData".to_owned(), nested)]));
        }

        let json = JsonValue::Object(BTreeMap::from([("TransactionMetaData".to_owned(), nested)]));
        assert!(STParsedJSONObject::new("test", &json).object.is_none());
    }
}
