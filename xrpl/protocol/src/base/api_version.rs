//! Narrow API-version compatibility helpers from `xrpl/protocol/ApiVersion.h`.

use std::collections::BTreeMap;

use crate::JsonValue;

pub const API_INVALID_VERSION: u32 = 0;
pub const API_MINIMUM_SUPPORTED_VERSION: u32 = 1;
pub const API_MAXIMUM_SUPPORTED_VERSION: u32 = 2;
pub const API_VERSION_IF_UNSPECIFIED: u32 = 1;
pub const API_COMMAND_LINE_VERSION: u32 = 1;
pub const API_BETA_VERSION: u32 = 3;
pub const API_MAXIMUM_VALID_VERSION: u32 = API_BETA_VERSION;

pub fn set_version(parent: &mut JsonValue, api_version: u32, beta_enabled: bool) {
    assert_ne!(
        api_version, API_INVALID_VERSION,
        "api_version input must be valid"
    );

    let JsonValue::Object(parent) = parent else {
        *parent = JsonValue::Object(BTreeMap::new());
        set_version(parent, api_version, beta_enabled);
        return;
    };

    let mut version = BTreeMap::new();

    if api_version == API_VERSION_IF_UNSPECIFIED {
        version.insert("first".to_owned(), JsonValue::String("1.0.0".to_owned()));
        version.insert("good".to_owned(), JsonValue::String("1.0.0".to_owned()));
        version.insert("last".to_owned(), JsonValue::String("1.0.0".to_owned()));
    } else {
        version.insert(
            "first".to_owned(),
            JsonValue::Unsigned(u64::from(API_MINIMUM_SUPPORTED_VERSION)),
        );
        version.insert(
            "last".to_owned(),
            JsonValue::Unsigned(u64::from(if beta_enabled {
                API_BETA_VERSION
            } else {
                API_MAXIMUM_SUPPORTED_VERSION
            })),
        );
    }

    parent.insert("version".to_owned(), JsonValue::Object(version));
}

pub fn get_api_version_number(json: &JsonValue, beta_enabled: bool) -> u32 {
    let max_version = if beta_enabled {
        API_BETA_VERSION
    } else {
        API_MAXIMUM_SUPPORTED_VERSION
    };

    let JsonValue::Object(object) = json else {
        return API_VERSION_IF_UNSPECIFIED;
    };

    let Some(specified) = object.get("api_version") else {
        return API_VERSION_IF_UNSPECIFIED;
    };

    let Some(specified) = (match specified {
        JsonValue::Signed(value) if *value >= 0 => u32::try_from(*value).ok(),
        JsonValue::Unsigned(value) => u32::try_from(*value).ok(),
        _ => None,
    }) else {
        return API_INVALID_VERSION;
    };

    if !(API_MINIMUM_SUPPORTED_VERSION..=max_version).contains(&specified) {
        API_INVALID_VERSION
    } else {
        specified
    }
}

pub fn for_api_versions<const MIN_VER: u32, const MAX_VER: u32>(mut f: impl FnMut(u32)) {
    let mut version = MIN_VER;
    while version <= MAX_VER {
        f(version);
        version += 1;
    }
}

pub fn for_all_api_versions(f: impl FnMut(u32)) {
    for_api_versions::<API_MINIMUM_SUPPORTED_VERSION, API_MAXIMUM_VALID_VERSION>(f);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        API_INVALID_VERSION, API_VERSION_IF_UNSPECIFIED, get_api_version_number, set_version,
    };
    use crate::JsonValue;

    #[test]
    fn get_api_version_ranges() {
        assert_eq!(
            get_api_version_number(
                &JsonValue::Object(BTreeMap::from([(
                    "api_version".to_owned(),
                    JsonValue::Unsigned(2),
                )])),
                false,
            ),
            2
        );
        assert_eq!(
            get_api_version_number(
                &JsonValue::Object(BTreeMap::from([(
                    "api_version".to_owned(),
                    JsonValue::String("2".to_owned()),
                )])),
                true,
            ),
            API_INVALID_VERSION
        );
        assert_eq!(
            get_api_version_number(&JsonValue::Object(BTreeMap::new()), true),
            API_VERSION_IF_UNSPECIFIED
        );
    }

    #[test]
    fn set_version_writes_current_v1_shape() {
        let mut json = JsonValue::Object(BTreeMap::new());
        set_version(&mut json, API_VERSION_IF_UNSPECIFIED, true);

        let JsonValue::Object(object) = json else {
            panic!("version response must be an object");
        };
        assert!(matches!(object.get("version"), Some(JsonValue::Object(_))));
    }
}
