use std::collections::BTreeMap;
use std::time::Duration;

use ledger::Ledger;
use protocol::{JsonValue, XRPAmount};

use crate::state::app_server_info_source::AppServerInfoView;

const HIGH_AGE_THRESHOLD_SECS: u64 = 1_000_000;

/// rippled `XRPAmount::decimalXRP()`: drops / 1e6 as a `double`.
fn decimal_xrp(drops: u64) -> JsonValue {
    crate::state::app_server_info_fee::json_double(drops as f64 / 1_000_000.0)
}

fn build_ledger_summary(ledger: &Ledger, human: bool) -> JsonValue {
    let header = ledger.header();
    let fees = ledger.fees();
    let mut summary = BTreeMap::new();
    summary.insert("seq".to_owned(), JsonValue::Unsigned(u64::from(header.seq)));
    summary.insert(
        "hash".to_owned(),
        JsonValue::String(header.hash.to_string()),
    );

    if human {
        summary.insert("base_fee_xrp".to_owned(), decimal_xrp(fees.base));
        summary.insert("reserve_base_xrp".to_owned(), decimal_xrp(fees.reserve));
        summary.insert("reserve_inc_xrp".to_owned(), decimal_xrp(fees.increment));
    } else {
        summary.insert(
            "base_fee".to_owned(),
            XRPAmount::from_drops(
                i64::try_from(fees.base).expect("base fee must fit within XRPAmount range"),
            )
            .json_clipped(),
        );
        summary.insert(
            "reserve_base".to_owned(),
            XRPAmount::from_drops(
                i64::try_from(fees.reserve).expect("reserve base must fit within XRPAmount range"),
            )
            .json_clipped(),
        );
        summary.insert(
            "reserve_inc".to_owned(),
            XRPAmount::from_drops(
                i64::try_from(fees.increment)
                    .expect("reserve increment must fit within XRPAmount range"),
            )
            .json_clipped(),
        );
        summary.insert(
            "close_time".to_owned(),
            JsonValue::Unsigned(u64::from(header.close_time)),
        );
    }

    JsonValue::Object(summary)
}

fn bounded_human_age(age: Duration) -> JsonValue {
    JsonValue::Unsigned(if age.as_secs() < HIGH_AGE_THRESHOLD_SECS {
        age.as_secs()
    } else {
        0
    })
}

fn human_ledger_age<V: AppServerInfoView>(
    view: &V,
    ledger: &Ledger,
    validated: bool,
) -> Option<Duration> {
    if validated {
        return Some(view.validated_ledger_age());
    }

    let ledger_close_time = ledger.header().close_time;
    let current_close_time = view.current_close_time_seconds();
    (ledger_close_time <= current_close_time)
        .then(|| Duration::from_secs(u64::from(current_close_time - ledger_close_time)))
}

pub fn append_ledger_fields<V: AppServerInfoView>(
    info: &mut BTreeMap<String, JsonValue>,
    view: &V,
    human: bool,
) {
    let Some((current, validated)) = view
        .validated_ledger()
        .map(|ledger| (ledger, true))
        .or_else(|| view.closed_ledger().map(|ledger| (ledger, false)))
    else {
        return;
    };

    let current_seq = current.header().seq;
    let field_name = if validated {
        "validated_ledger"
    } else {
        "closed_ledger"
    };
    let age = human
        .then(|| human_ledger_age(view, current.as_ref(), validated))
        .flatten();
    let mut summary = build_ledger_summary(current.as_ref(), human);
    if human && view.close_time_offset_seconds().unsigned_abs() >= 60 {
        let JsonValue::Object(summary_object) = &mut summary else {
            unreachable!("ledger summary must always be an object");
        };
        summary_object.insert(
            "close_time_offset".to_owned(),
            JsonValue::Unsigned(u64::from(view.close_time_offset_seconds() as u32)),
        );
    }
    if human {
        let JsonValue::Object(summary_object) = &mut summary else {
            unreachable!("ledger summary must always be an object");
        };
        if let Some(age) = age {
            summary_object.insert("age".to_owned(), bounded_human_age(age));
        }
    }
    info.insert(field_name.to_owned(), summary);

    match view.published_ledger() {
        None => {
            info.insert(
                "published_ledger".to_owned(),
                JsonValue::String("none".to_owned()),
            );
        }
        Some(published) if published.header().seq != current_seq => {
            info.insert(
                "published_ledger".to_owned(),
                JsonValue::Unsigned(u64::from(published.header().seq)),
            );
        }
        Some(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{append_ledger_fields, decimal_xrp};
    use app::ApplicationRoot;
    use ledger::{Fees, Ledger, LedgerHeader};
    use protocol::JsonValue;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn sample_ledger(seq: u32, close_time: u32, hash_byte: u8) -> Arc<Ledger> {
        let mut ledger = Ledger::from_ledger_seq_and_close_time(seq, close_time, false);
        ledger.set_ledger_info(LedgerHeader {
            hash: basics::sha_map_hash::SHAMapHash::new(basics::base_uint::Uint256::from_array(
                [hash_byte; 32],
            )),
            ..ledger.header()
        });
        ledger.set_fees(Fees {
            base: 10,
            reserve: 2_000_000,
            increment: 200_000,
        });
        Arc::new(ledger)
    }

    /// rippled `decimalXRP()` is a `double`, so human server_info sends JSON
    /// numbers (`1e-05`, `0.2`), not strings.
    #[test]
    fn human_xrp_amounts_are_json_numbers() {
        let raw = |drops| match decimal_xrp(drops) {
            JsonValue::Raw(raw) => raw.as_str().to_owned(),
            other => panic!("expected a raw JSON number, got {other:?}"),
        };
        assert_eq!(raw(1_000_000), "1");
        assert_eq!(raw(2_500_000), "2.5");
        assert_eq!(raw(1_234_560), "1.23456");
        assert_eq!(raw(10), "1e-05");
        assert_eq!(raw(200_000), "0.2");
    }

    #[test]
    fn app_server_info_ledger_fields_follow_validated_and_published_contract() {
        let app = ApplicationRoot::new(0).expect("root shell should build");
        let closed = sample_ledger(100, 1_000, 0x11);
        let validated = sample_ledger(101, 1_005, 0x22);
        let published = sample_ledger(99, 999, 0x33);
        app.on_closed_ledger(closed);
        app.on_validated_ledger(validated);
        app.on_published_ledger(published);
        let now_close_time = app.time_keeper().close_time().as_seconds();
        app.ledger_master_state()
            .set_validated_close_time(now_close_time.saturating_sub(20));

        let mut info = BTreeMap::new();
        append_ledger_fields(&mut info, &app, false);

        assert!(matches!(
            info.get("validated_ledger"),
            Some(JsonValue::Object(_))
        ));
        assert_eq!(info.get("published_ledger"), Some(&JsonValue::Unsigned(99)));
    }

    #[test]
    fn app_server_info_human_ledger_fields_follow_cpp_age_and_offset_rules() {
        let app = ApplicationRoot::new(0).expect("root shell should build");
        assert_eq!(
            app.time_keeper()
                .adjust_close_time(time::Duration::seconds(240)),
            time::Duration::seconds(60)
        );
        let closed = sample_ledger(
            100,
            app.current_close_time_seconds().saturating_sub(15),
            0x11,
        );
        app.on_closed_ledger(closed);

        let mut info = BTreeMap::new();
        append_ledger_fields(&mut info, &app, true);

        let JsonValue::Object(closed_ledger) =
            info.get("closed_ledger").expect("closed ledger must exist")
        else {
            panic!("closed ledger must be an object");
        };

        assert_eq!(
            closed_ledger.get("close_time_offset"),
            Some(&JsonValue::Unsigned(60))
        );
        assert_eq!(closed_ledger.get("age"), Some(&JsonValue::Unsigned(15)));
    }
}
