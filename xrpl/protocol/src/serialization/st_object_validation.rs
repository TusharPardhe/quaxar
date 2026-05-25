//! `protocol_autogen/STObjectValidation.h` port.

use crate::{Asset, SOETxMPTIssue, SOTemplate, STIssue, STObject};

pub fn validate_st_object(object: &STObject, format: &SOTemplate) -> bool {
    for field in format.iter() {
        if !object.is_field_present(field.sfield()) {
            if field.style() == crate::SOEStyle::Required {
                return false;
            }
            continue;
        }

        if field.support_mpt() != SOETxMPTIssue::NotSupported {
            continue;
        }

        match field.sfield().field_type() {
            crate::SerializedTypeId::Amount => {
                if object.get_field_amount(field.sfield()).holds_mpt_issue() {
                    return false;
                }
            }
            crate::SerializedTypeId::Issue => {
                let Some(issue) = object.peek_at_pfield(field.sfield()) else {
                    return false;
                };
                let Some(issue) = issue.as_any().downcast_ref::<STIssue>() else {
                    return false;
                };
                if matches!(issue.asset(), Asset::MPTIssue(_)) {
                    return false;
                }
            }
            _ => {}
        }
    }

    true
}
