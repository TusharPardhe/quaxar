use basics::basic_config::BasicConfig;
use tx::{TXQ_BASE_LEVEL, TxQSetup, TxQSetupError, setup_txq};

fn config_with_transaction_queue() -> BasicConfig {
    let mut config = BasicConfig::new();
    let section = config.section_mut("transaction_queue");
    section.set("ledgers_in_queue", "25");
    section.set("minimum_queue_size", "4096");
    section.set("retry_sequence_percent", "40");
    section.set("minimum_escalation_multiplier", "999999");
    section.set("minimum_txn_in_ledger", "44");
    section.set("minimum_txn_in_ledger_standalone", "1444");
    section.set("target_txn_in_ledger", "333");
    section.set("maximum_txn_in_ledger", "5000");
    section.set("normal_consensus_increase_percent", "5005");
    section.set("slow_consensus_decrease_percent", "400");
    section.set("maximum_txn_per_account", "21");
    section.set("minimum_last_ledger_buffer", "9");
    config
}

#[test]
fn setup_txq_uses_cpp_defaults_when_section_is_missing() {
    let config = BasicConfig::new();

    let setup = setup_txq(&config, false).expect("default config should be valid");

    assert_eq!(setup, TxQSetup::default());
    assert_eq!(setup.minimum_txn_in_ledger_for_mode(), 32);
    assert_eq!(setup.minimum_escalation_multiplier, TXQ_BASE_LEVEL * 500);
    assert_eq!(setup.fee_metrics_config().minimum_txn_in_ledger, 32);
}

#[test]
fn setup_txq_reads_overrides_and_clamps_consensus_fields() {
    let config = config_with_transaction_queue();

    let setup = setup_txq(&config, true).expect("configured values should be valid");

    assert_eq!(
        setup,
        TxQSetup {
            ledgers_in_queue: 25,
            queue_size_min: 4096,
            retry_sequence_percent: 40,
            minimum_escalation_multiplier: 999999,
            minimum_txn_in_ledger: 44,
            minimum_txn_in_ledger_standalone: 1444,
            target_txn_in_ledger: 333,
            maximum_txn_in_ledger: Some(5000),
            normal_consensus_increase_percent: 1000,
            slow_consensus_decrease_percent: 100,
            maximum_txn_per_account: 21,
            minimum_last_ledger_buffer: 9,
            standalone: true,
        }
    );
    assert_eq!(setup.minimum_txn_in_ledger_for_mode(), 1444);
    assert_eq!(setup.fee_metrics_config().minimum_txn_in_ledger, 1444);
}

#[test]
fn setup_txq_rejects_maximum_below_normal_minimum() {
    let mut config = BasicConfig::new();
    let section = config.section_mut("transaction_queue");
    section.set("minimum_txn_in_ledger", "40");
    section.set("minimum_txn_in_ledger_standalone", "1000");
    section.set("maximum_txn_in_ledger", "39");

    let error = setup_txq(&config, false).expect_err("maximum below normal minimum must fail");

    assert_eq!(error, TxQSetupError::MaximumBelowMinimumTxnInLedger);
    assert_eq!(
        error.to_string(),
        "The minimum number of low-fee transactions allowed per ledger \
(minimum_txn_in_ledger) exceeds the maximum number of low-fee transactions \
allowed per ledger (maximum_txn_in_ledger)."
    );
}

#[test]
fn setup_txq_rejects_maximum_below_standalone_minimum() {
    let mut config = BasicConfig::new();
    let section = config.section_mut("transaction_queue");
    section.set("minimum_txn_in_ledger", "32");
    section.set("minimum_txn_in_ledger_standalone", "1000");
    section.set("maximum_txn_in_ledger", "999");

    let error = setup_txq(&config, false).expect_err("maximum below standalone minimum must fail");

    assert_eq!(
        error,
        TxQSetupError::MaximumBelowMinimumTxnInLedgerStandalone
    );
    assert_eq!(
        error.to_string(),
        "The minimum number of low-fee transactions allowed per ledger \
(minimum_txn_in_ledger_standalone) exceeds the maximum number of low-fee \
transactions allowed per ledger (maximum_txn_in_ledger)."
    );
}
