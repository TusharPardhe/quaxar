use std::collections::BTreeMap;

use app::{ValidatorListExpiration, ValidatorListStatus, ValidatorListStatusSnapshot};
use basics::chrono::{NetClockTimePoint, to_string};
use protocol::JsonValue;

use crate::state::app_server_info_source::AppServerInfoView;

fn validator_list_status_string(status: ValidatorListStatus) -> String {
    match status {
        ValidatorListStatus::Active => "active".to_owned(),
        ValidatorListStatus::Expired => "expired".to_owned(),
        ValidatorListStatus::Unknown => "unknown".to_owned(),
    }
}

fn validator_list_expiration_string(expiration: ValidatorListExpiration) -> String {
    match expiration {
        ValidatorListExpiration::Unknown => "unknown".to_owned(),
        ValidatorListExpiration::Never => "never".to_owned(),
        ValidatorListExpiration::Seconds(seconds) => to_string(NetClockTimePoint::new(seconds)),
    }
}

fn validator_list_expiration_seconds(expiration: ValidatorListExpiration) -> u64 {
    match expiration {
        ValidatorListExpiration::Unknown => 0,
        ValidatorListExpiration::Never => u64::from(u32::MAX),
        ValidatorListExpiration::Seconds(seconds) => u64::from(seconds),
    }
}

fn human_validator_list(snapshot: ValidatorListStatusSnapshot) -> JsonValue {
    JsonValue::Object(BTreeMap::from([
        (
            "count".to_owned(),
            JsonValue::Unsigned(snapshot.count as u64),
        ),
        (
            "expiration".to_owned(),
            JsonValue::String(validator_list_expiration_string(snapshot.expiration)),
        ),
        (
            "status".to_owned(),
            JsonValue::String(validator_list_status_string(snapshot.status)),
        ),
    ]))
}

pub(crate) fn append_validator_fields<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    view: &V,
    human: bool,
    admin: bool,
) {
    let validators = view.validators();
    info.insert(
        "validation_quorum".to_owned(),
        JsonValue::Unsigned(validators.quorum() as u64),
    );

    if !admin {
        return;
    }

    let validator_list_status = view.validator_list_status_snapshot();

    if human {
        info.insert(
            "validator_list".to_owned(),
            human_validator_list(validator_list_status),
        );
    } else {
        info.insert(
            "validator_list_expires".to_owned(),
            JsonValue::Unsigned(validator_list_expiration_seconds(
                validator_list_status.expiration,
            )),
        );
    }

    info.insert(
        "pubkey_validator".to_owned(),
        JsonValue::String(view.admin_pubkey_validator()),
    );
}

#[cfg(test)]
mod tests {
    use super::append_validator_fields;
    use app::ApplicationRoot;
    use protocol::{JsonValue, PublicKey};
    use std::collections::BTreeMap;

    #[test]
    fn validator_fields_shape_human_admin_summary() {
        let app = ApplicationRoot::new(0).expect("root shell should build");
        let mut info = BTreeMap::new();

        append_validator_fields(&mut info, &app, true, true);

        assert_eq!(info.get("validation_quorum"), Some(&JsonValue::Unsigned(1)));
        let JsonValue::Object(validator_list) = info
            .get("validator_list")
            .expect("validator_list should exist")
        else {
            panic!("validator_list should be object");
        };
        assert_eq!(validator_list.get("count"), Some(&JsonValue::Unsigned(0)));
        assert_eq!(
            validator_list.get("expiration"),
            Some(&JsonValue::String("unknown".to_owned()))
        );
        assert_eq!(
            validator_list.get("status"),
            Some(&JsonValue::String("unknown".to_owned()))
        );
        assert!(!validator_list.contains_key("validator_list_threshold"));
        assert_eq!(
            info.get("pubkey_validator"),
            Some(&JsonValue::String("none".to_owned()))
        );
        assert!(!info.contains_key("validator_list_expires"));
    }

    #[test]
    fn validator_fields_shape_non_human_admin_expiry_and_pubkey() {
        let mut app = ApplicationRoot::new(0).expect("root shell should build");
        let local_signing_key = PublicKey::from_bytes([0x02; 33]);
        assert!(app.validators().load(
            Some(local_signing_key),
            &["n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7".to_owned()],
            &[],
            None,
        ));
        app.set_validation_public_key(PublicKey::from_bytes([0x03; 33]));
        let local_public_key = app
            .validators()
            .local_public_key()
            .expect("local validator key should be set");

        let mut info = BTreeMap::new();
        append_validator_fields(&mut info, &app, false, true);

        assert_eq!(info.get("validation_quorum"), Some(&JsonValue::Unsigned(1)));
        assert_eq!(
            info.get("validator_list_expires"),
            Some(&JsonValue::Unsigned(u32::MAX as u64))
        );
        assert_eq!(
            info.get("pubkey_validator"),
            Some(&JsonValue::String(local_public_key.to_node_public_base58()))
        );
        assert!(!info.contains_key("validator_list"));
    }

    #[test]
    fn validator_fields_keep_admin_only_branches_hidden_from_non_admin() {
        let app = ApplicationRoot::new(0).expect("root shell should build");
        let mut info = BTreeMap::new();

        append_validator_fields(&mut info, &app, true, false);

        assert_eq!(info.get("validation_quorum"), Some(&JsonValue::Unsigned(1)));
        assert!(!info.contains_key("validator_list"));
        assert!(!info.contains_key("validator_list_expires"));
        assert!(!info.contains_key("pubkey_validator"));
    }
}
