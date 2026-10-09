//! A populated standalone ledger and the heavy ledger-object RPC calls,
//! shared by the raw-rendering parity tests and the endpoint benchmark
//! (`examples/endpoint_render.rs`).

use protocol::{
    currency_from_string, get_field_by_symbol, to_base58, Issue, JsonValue, STAmount, STTx, TxType,
};

use crate::env::{json, sign_tx, RpcTestEnv, TestAccount};

pub struct RenderScenario {
    pub env: RpcTestEnv,
    pub gateway: TestAccount,
    pub holder: TestAccount,
    pub offers: usize,
    pub tickets: usize,
    /// A committed OfferCreate (for `tx`).
    pub sample_tx: basics::base_uint::Uint256,
}

fn fee() -> STAmount {
    STAmount::new_native(10, false)
}

/// `offers` gateway offers selling its own USD at distinct XRP prices (one
/// order book) and `tickets` Tickets owned by `holder`.
pub fn build(offers: usize, tickets: usize) -> RenderScenario {
    let mut gateway = TestAccount::new("render_gateway");
    let mut holder = TestAccount::new("render_holder");
    let env = RpcTestEnv::new(&[(&gateway, 100_000_000_000), (&holder, 100_000_000_000)]);

    let usd = currency_from_string("USD");
    let mut txs = Vec::with_capacity(offers + 1);
    for i in 0..offers {
        let mut offer = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), gateway.id);
            tx.set_field_amount(
                get_field_by_symbol("sfTakerPays"),
                STAmount::new_native(1_000_000 + 1_000 * i as u64, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfTakerGets"),
                STAmount::new_with_asset(
                    get_field_by_symbol("sfTakerGets"),
                    Issue::new(usd, gateway.id),
                    1_234_567_890_123_456,
                    -15,
                    false,
                ),
            );
            tx.set_field_amount(get_field_by_symbol("sfFee"), fee());
            tx.set_field_u32(get_field_by_symbol("sfSequence"), gateway.next_seq());
        });
        sign_tx(&mut offer, &gateway);
        txs.push(offer);
    }
    let mut remaining = tickets;
    while remaining > 0 {
        let count = remaining.min(250);
        remaining -= count;
        let mut ticket = STTx::new(TxType::TICKET_CREATE, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), holder.id);
            tx.set_field_u32(get_field_by_symbol("sfTicketCount"), count as u32);
            tx.set_field_amount(get_field_by_symbol("sfFee"), fee());
            tx.set_field_u32(get_field_by_symbol("sfSequence"), holder.next_seq());
        });
        sign_tx(&mut ticket, &holder);
        txs.push(ticket);
        // Tickets consume sequence numbers.
        for _ in 0..count {
            holder.next_seq();
        }
    }
    let sample_tx = txs[0].get_transaction_id();
    // Close every few transactions: the open ledger only queues a bounded
    // number of transactions per account per ledger.
    for chunk in txs.chunks(8) {
        let refs: Vec<&STTx> = chunk.iter().collect();
        env.submit_all_and_close(&refs);
    }

    RenderScenario {
        env,
        gateway,
        holder,
        offers,
        tickets,
        sample_tx,
    }
}

/// The endpoints whose results embed ledger objects.
pub const ENDPOINTS: [&str; 8] = [
    "book_offers",
    "account_objects",
    "account_info",
    "ledger_data",
    "tx",
    "tx_v1",
    "account_tx",
    "ledger_expand",
];

/// Call `endpoint` against the scenario and return the RPC result.
pub fn call(scenario: &RenderScenario, endpoint: &str) -> JsonValue {
    let source = scenario.env.rpc_source();
    match endpoint {
        "book_offers" => rpc::do_book_offers(
            &rpc::BookOffersRequest {
                params: &json([
                    (
                        "taker_pays",
                        json([("currency", JsonValue::String("XRP".into()))]),
                    ),
                    (
                        "taker_gets",
                        json([
                            ("currency", JsonValue::String("USD".into())),
                            ("issuer", JsonValue::String(to_base58(scenario.gateway.id))),
                        ]),
                    ),
                    ("limit", JsonValue::Unsigned(scenario.offers as u64)),
                ]),
                api_version: 2,
                role: rpc::Role::Admin,
            },
            &source,
            &source,
        ),
        "account_objects" => rpc::do_account_objects(
            &rpc::AccountObjectsRequest {
                params: &json([
                    ("account", JsonValue::String(to_base58(scenario.holder.id))),
                    ("limit", JsonValue::Unsigned(400)),
                ]),
                api_version: 2,
                role: rpc::Role::Admin,
            },
            &source,
        ),
        "account_info" => rpc::do_account_info(
            &rpc::AccountInfoRequest {
                params: &json([
                    ("account", JsonValue::String(to_base58(scenario.gateway.id))),
                    ("signer_lists", JsonValue::Bool(true)),
                ]),
                api_version: 1,
                role: rpc::Role::Admin,
            },
            &source,
        ),
        "ledger_data" => match rpc::do_ledger_data(
            &rpc::LedgerDataRequest {
                params: &json([("limit", JsonValue::Unsigned(256))]),
                api_version: 2,
                role: rpc::Role::Admin,
            },
            &source,
        ) {
            rpc::LedgerDataResponse::Json(json) => json,
            rpc::LedgerDataResponse::PreRendered(bytes) => {
                JsonValue::Raw(protocol::RawJson::from_trusted_bytes(bytes.to_vec()))
            }
        },
        "tx" | "tx_v1" => rpc::do_tx(
            &rpc::TxRequest {
                params: &json([(
                    "transaction",
                    JsonValue::String(scenario.sample_tx.to_string()),
                )]),
                api_version: if endpoint == "tx" { 2 } else { 1 },
            },
            &source,
        ),
        "account_tx" => rpc::do_account_tx(
            &json([
                ("account", JsonValue::String(to_base58(scenario.gateway.id))),
                ("ledger_index_min", JsonValue::Signed(-1)),
                ("ledger_index_max", JsonValue::Signed(-1)),
                ("limit", JsonValue::Unsigned(200)),
            ]),
            rpc::RpcRole::Admin,
            2,
            &source,
        ),
        "ledger_expand" => rpc::do_ledger(
            &json([
                // The first closed ledger holds the first batch of offers.
                ("ledger_index", JsonValue::Unsigned(2)),
                ("transactions", JsonValue::Bool(true)),
                ("expand", JsonValue::Bool(true)),
            ]),
            rpc::RpcRole::Admin,
            2,
            &source,
        ),
        other => panic!("unknown endpoint {other}"),
    }
}

/// Result bytes as the server serializes them, rendering ledger objects with
/// trees (`raw = false`) or directly (`raw = true`).
pub fn render(scenario: &RenderScenario, endpoint: &str, raw: bool) -> Vec<u8> {
    let result = if raw {
        protocol::json_writer::with_raw_rendering(|| call(scenario, endpoint))
    } else {
        call(scenario, endpoint)
    };
    sonic_rs::to_vec(&result).expect("result serializes")
}
