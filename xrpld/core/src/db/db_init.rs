use crate::{DatabaseConSetup, StartUpType};
use basics::basic_config::{BasicConfig, set};
use std::path::PathBuf;

pub const COMMON_DB_PRAGMA_JOURNAL: &str = "PRAGMA journal_mode=%s;";
pub const COMMON_DB_PRAGMA_SYNC: &str = "PRAGMA synchronous=%s;";
pub const COMMON_DB_PRAGMA_TEMP: &str = "PRAGMA temp_store=%s;";
pub const SQLITE_TUNING_CUTOFF: u32 = 10_000_000;

pub const LEDGER_DB_NAME: &str = "ledger.db";
pub const LEDGER_DB_INIT: &[&str] = &[
    "BEGIN TRANSACTION;",
    "CREATE TABLE IF NOT EXISTS Ledgers (\
        LedgerHash      CHARACTER(64) PRIMARY KEY,\
        LedgerSeq       BIGINT UNSIGNED,\
        PrevHash        CHARACTER(64),\
        TotalCoins      BIGINT UNSIGNED,\
        ClosingTime     BIGINT UNSIGNED,\
        PrevClosingTime BIGINT UNSIGNED,\
        CloseTimeRes    BIGINT UNSIGNED,\
        CloseFlags      BIGINT UNSIGNED,\
        AccountSetHash  CHARACTER(64),\
        TransSetHash    CHARACTER(64)\
    );",
    "CREATE INDEX IF NOT EXISTS SeqLedger ON Ledgers(LedgerSeq);",
    "DROP TABLE IF EXISTS Validations;",
    "END TRANSACTION;",
];

pub const TRANSACTION_DB_NAME: &str = "transaction.db";
pub const TRANSACTION_DB_INIT: &[&str] = &[
    "BEGIN TRANSACTION;",
    "CREATE TABLE IF NOT EXISTS Transactions (\
        TransID     CHARACTER(64) PRIMARY KEY,\
        TransType   CHARACTER(24),\
        FromAcct    CHARACTER(35),\
        FromSeq     BIGINT UNSIGNED,\
        LedgerSeq   BIGINT UNSIGNED,\
        Status      CHARACTER(1),\
        RawTxn      BLOB,\
        TxnMeta     BLOB\
    );",
    "CREATE INDEX IF NOT EXISTS TxLgrIndex ON Transactions(LedgerSeq);",
    "CREATE TABLE IF NOT EXISTS AccountTransactions (\
        TransID     CHARACTER(64),\
        Account     CHARACTER(64),\
        LedgerSeq   BIGINT UNSIGNED,\
        TxnSeq      INTEGER\
    );",
    "CREATE INDEX IF NOT EXISTS AcctTxIDIndex ON AccountTransactions(TransID);",
    "CREATE INDEX IF NOT EXISTS AcctTxIndex ON AccountTransactions(Account, LedgerSeq, TxnSeq, TransID);",
    "CREATE INDEX IF NOT EXISTS AcctLgrIndex ON AccountTransactions(LedgerSeq, Account, TransID);",
    "END TRANSACTION;",
];

pub const WALLET_DB_NAME: &str = "wallet.db";
pub const WALLET_DB_INIT: &[&str] = &[
    "BEGIN TRANSACTION;",
    "CREATE TABLE IF NOT EXISTS NodeIdentity (\
        PublicKey       CHARACTER(53),\
        PrivateKey      CHARACTER(52)\
    );",
    "CREATE TABLE IF NOT EXISTS PeerReservations (\
        PublicKey       CHARACTER(53) UNIQUE NOT NULL,\
        Description     CHARACTER(64) NOT NULL\
    );",
    "CREATE TABLE IF NOT EXISTS ValidatorManifests (\
        RawData          BLOB NOT NULL\
    );",
    "CREATE TABLE IF NOT EXISTS PublisherManifests (\
        RawData          BLOB NOT NULL\
    );",
    "END TRANSACTION;",
];

pub fn build_database_con_setup(
    config: &BasicConfig,
    start_up: StartUpType,
    stand_alone: bool,
    ledger_history: u32,
) -> Result<DatabaseConSetup, String> {
    let data_dir = config
        .legacy("database_path")
        .map_err(|error| error.to_string())?;
    if !stand_alone && data_dir.is_empty() {
        return Err("database_path must be set.".to_owned());
    }

    let sqlite = config.section("sqlite");
    let mut safety_level = String::new();
    let mut journal_mode = "wal".to_owned();
    let mut synchronous = "normal".to_owned();
    let mut temp_store = "file".to_owned();
    let mut show_risk_warning = false;

    if set(&mut safety_level, "safety_level", sqlite) {
        if safety_level.eq_ignore_ascii_case("low") {
            journal_mode = "memory".to_owned();
            synchronous = "off".to_owned();
            temp_store = "memory".to_owned();
            show_risk_warning = true;
        } else if !safety_level.eq_ignore_ascii_case("high") {
            return Err(format!("Invalid safety_level value: {safety_level}"));
        }
    }

    let mut configured_journal_mode = journal_mode.clone();
    if set(&mut configured_journal_mode, "journal_mode", sqlite) {
        if !safety_level.is_empty() {
            return Err(
                "Configuration file may not define both \"safety_level\" and \"journal_mode\""
                    .to_owned(),
            );
        }
        journal_mode = configured_journal_mode;
    }
    let journal_lower = journal_mode.to_ascii_lowercase();
    let journal_higher_risk = matches!(journal_lower.as_str(), "memory" | "off");
    show_risk_warning |= journal_higher_risk;
    if !journal_higher_risk
        && !matches!(
            journal_lower.as_str(),
            "delete" | "truncate" | "persist" | "wal"
        )
    {
        return Err(format!("Invalid journal_mode value: {journal_mode}"));
    }

    let mut configured_synchronous = synchronous.clone();
    if set(&mut configured_synchronous, "synchronous", sqlite) {
        if !safety_level.is_empty() {
            return Err(
                "Configuration file may not define both \"safety_level\" and \"synchronous\""
                    .to_owned(),
            );
        }
        synchronous = configured_synchronous;
    }
    let synchronous_lower = synchronous.to_ascii_lowercase();
    let sync_higher_risk = synchronous_lower == "off";
    show_risk_warning |= sync_higher_risk;
    if !sync_higher_risk && !matches!(synchronous_lower.as_str(), "normal" | "full" | "extra") {
        return Err(format!("Invalid synchronous value: {synchronous}"));
    }

    let mut configured_temp_store = temp_store.clone();
    if set(&mut configured_temp_store, "temp_store", sqlite) {
        if !safety_level.is_empty() {
            return Err(
                "Configuration file may not define both \"safety_level\" and \"temp_store\""
                    .to_owned(),
            );
        }
        temp_store = configured_temp_store;
    }
    let temp_store_lower = temp_store.to_ascii_lowercase();
    let temp_higher_risk = temp_store_lower == "memory";
    show_risk_warning |= temp_higher_risk;
    if !temp_higher_risk && !matches!(temp_store_lower.as_str(), "default" | "file") {
        return Err(format!("Invalid temp_store value: {temp_store}"));
    }

    let common_pragma = vec![
        pragma_with_value(COMMON_DB_PRAGMA_JOURNAL, &journal_mode),
        pragma_with_value(COMMON_DB_PRAGMA_SYNC, &synchronous),
        pragma_with_value(COMMON_DB_PRAGMA_TEMP, &temp_store),
    ];

    let mut journal_size_limit = 1_582_080_i64;
    let mut page_size = 4_096_i64;
    if config.exists("sqlite") {
        let sqlite = config.section("sqlite");
        set(&mut journal_size_limit, "journal_size_limit", sqlite);
        set(&mut page_size, "page_size", sqlite);

        if !(512..=65_536).contains(&page_size) {
            return Err("Invalid page_size. Must be between 512 and 65536.".to_owned());
        }

        if (page_size & (page_size - 1)) != 0 {
            return Err("Invalid page_size. Must be a power of 2.".to_owned());
        }
    }

    let _ = show_risk_warning && ledger_history > SQLITE_TUNING_CUTOFF;

    Ok(DatabaseConSetup {
        start_up,
        stand_alone,
        data_dir: PathBuf::from(data_dir),
        common_pragma: Some(common_pragma),
        tx_pragma: vec![
            numeric_pragma("page_size", page_size),
            numeric_pragma("journal_size_limit", journal_size_limit),
            numeric_pragma("max_page_count", 4_294_967_294),
            numeric_pragma("mmap_size", 17_179_869_184),
        ],
        lgr_pragma: vec![numeric_pragma("journal_size_limit", 1_582_080)],
    })
}

fn pragma_with_value(template: &str, value: &str) -> String {
    template.replacen("%s", value, 1)
}

fn numeric_pragma(key: &str, value: i64) -> String {
    format!("PRAGMA {key}={value};")
}

#[cfg(test)]
mod tests {
    use super::build_database_con_setup;
    use crate::{
        COMMON_DB_PRAGMA_JOURNAL, COMMON_DB_PRAGMA_SYNC, COMMON_DB_PRAGMA_TEMP, StartUpType,
    };
    use basics::basic_config::BasicConfig;

    #[test]
    fn setup_default_sqlite_pragmas() {
        let mut config = BasicConfig::new();
        config.set_legacy("database_path", "/tmp/xrpld");

        let setup =
            build_database_con_setup(&config, StartUpType::Normal, false, 256).expect("setup");

        assert_eq!(
            setup.common_pragma,
            Some(vec![
                COMMON_DB_PRAGMA_JOURNAL.replacen("%s", "wal", 1),
                COMMON_DB_PRAGMA_SYNC.replacen("%s", "normal", 1),
                COMMON_DB_PRAGMA_TEMP.replacen("%s", "file", 1),
            ])
        );
        assert_eq!(setup.tx_pragma[0], "PRAGMA page_size=4096;");
        assert_eq!(setup.tx_pragma[1], "PRAGMA journal_size_limit=1582080;");
        assert_eq!(setup.lgr_pragma[0], "PRAGMA journal_size_limit=1582080;");
    }

    #[test]
    fn setup_applies_low_safety_level_and_page_size_validation() {
        let mut config = BasicConfig::new();
        config.set_legacy("database_path", "/tmp/xrpld");
        let sqlite = config.section_mut("sqlite");
        sqlite.set("safety_level", "low");
        sqlite.set("page_size", "1024");

        let setup =
            build_database_con_setup(&config, StartUpType::Normal, false, 256).expect("setup");
        assert_eq!(
            setup.common_pragma.as_ref().expect("pragma")[0],
            "PRAGMA journal_mode=memory;"
        );
        assert_eq!(setup.tx_pragma[0], "PRAGMA page_size=1024;");
    }

    #[test]
    fn setup_rejects_conflicting_or_invalid_sqlite_values() {
        let mut conflict = BasicConfig::new();
        conflict.set_legacy("database_path", "/tmp/xrpld");
        let sqlite = conflict.section_mut("sqlite");
        sqlite.set("safety_level", "low");
        sqlite.set("journal_mode", "wal");
        let error = build_database_con_setup(&conflict, StartUpType::Normal, false, 256)
            .expect_err("conflict should fail");
        assert_eq!(
            error,
            "Configuration file may not define both \"safety_level\" and \"journal_mode\""
        );

        let mut invalid = BasicConfig::new();
        invalid.set_legacy("database_path", "/tmp/xrpld");
        invalid.section_mut("sqlite").set("page_size", "1000");
        let error = build_database_con_setup(&invalid, StartUpType::Normal, false, 256)
            .expect_err("page size should fail");
        assert_eq!(error, "Invalid page_size. Must be a power of 2.");
    }
}
