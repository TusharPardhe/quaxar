use ledger::ApplyView;
use protocol::{STTx, Ter, feature_id};

pub fn apply_vault_create<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }
    // XLS-65: implementation would go here
    Ter::TEC_INTERNAL
}

pub fn apply_vault_set<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_vault_delete<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_vault_deposit<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_vault_withdraw<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_vault_clawback<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_loan_set<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    // XLS-66: implementation would go here
    Ter::TEC_INTERNAL
}

pub fn apply_loan_delete<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_loan_manage<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_loan_pay<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_loan_broker_set<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_loan_broker_delete<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_loan_broker_cover_deposit<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_loan_broker_cover_withdraw<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}

pub fn apply_loan_broker_cover_clawback<V: ApplyView>(view: &mut V, _sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }
    Ter::TEC_INTERNAL
}
