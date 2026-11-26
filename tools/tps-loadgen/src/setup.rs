//! Build the test accounts and the market on top of the forked ledger.
//!
//! Every phase reads the ledger, submits only what is missing, waits for
//! validation and reads again, so `setup` can be rerun after a failure.

use crate::accounts::{self, CURRENCIES, FIRST_HUB, HUBS, ISSUER, Key, Layout, MARKET_MAKER};
use crate::node::{self, Pool, accepted, engine_result};
use crate::tx::{self, Common, Signed, currency, iou, xrp};
use futures::stream::{FuturesUnordered, StreamExt};
use protocol::{AccountID, to_base58};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A transaction still to be signed: the account it is sent from (for
/// routing), its sequence and the builder that signs it with the fee and
/// last ledger current at send time.
pub type Item = (AccountID, u32, Box<dyn Fn(Common) -> Signed + Send + Sync>);

/// Highest fee setup pays for one transaction (10 XRP).
const SETUP_MAX_FEE: u64 = 10_000_000;

pub const TEST_XRP: u64 = 200_000_000; // 200 XRP per test account
const ISSUER_XRP: u64 = 1_000_000_000;
const MM_XRP: u64 = 12_000_000_000_000; // 12M XRP
/// XRP side of each XRP/currency pool, and each IOU pool side, in units.
pub const POOL_XRP: u64 = 1_000_000_000_000; // 1M XRP
const POOL_IOU_EXP: i32 = 6; // 1e6 units
const OFFER_IOU_EXP: i32 = 5; // 1e5 units per offer
const OFFER_XRP: u64 = 100_000_000_000; // 1e5 XRP per offer
const LSF_DEFAULT_RIPPLE: u32 = 0x0080_0000;
const ASF_DEFAULT_RIPPLE: u32 = 8;
const MAX_ROUNDS: usize = 6;
/// AMMCreate pays at least the owner reserve increment as its fee.
const AMM_CREATE_FEE: u64 = 2_000_000;

pub struct Setup {
    pub pool: Pool,
    pub tag: String,
    pub funder: AccountID,
    pub layout: Layout,
    pub rate: u64,
}

struct Keys {
    funder_key: Arc<Key>,
    issuer: Arc<Key>,
    mm: Arc<Key>,
    hubs: Vec<Arc<Key>>,
    tests: Vec<Arc<Key>>,
}

impl Setup {
    pub async fn run(&self) -> Result<(), String> {
        let t0 = Instant::now();
        let keys = Keys {
            funder_key: Arc::new(accounts::derive(&self.tag, accounts::FUNDER_KEY)),
            issuer: Arc::new(accounts::derive(&self.tag, ISSUER)),
            mm: Arc::new(accounts::derive(&self.tag, MARKET_MAKER)),
            hubs: accounts::derive_range(&self.tag, FIRST_HUB..FIRST_HUB + HUBS).into_iter().map(Arc::new).collect(),
            tests: accounts::derive_range(
                &self.tag,
                accounts::FIRST_TEST..accounts::FIRST_TEST + self.layout.tests,
            )
            .into_iter()
            .map(Arc::new)
            .collect(),
        };
        log(&format!("derived {} keys in {:.1}s", keys.tests.len() + HUBS as usize + 3, t0.elapsed().as_secs_f64()));

        self.repeat("core accounts", || self.fund_core(&keys)).await?;
        self.repeat("test accounts", || self.fund_tests(&keys)).await?;
        self.repeat("issuer DefaultRipple", || self.default_ripple(&keys)).await?;
        self.repeat("trust lines", || self.trust_lines(&keys)).await?;
        self.repeat("issued balances", || self.issue(&keys)).await?;
        self.repeat("AMM pools", || self.amms(&keys)).await?;
        self.repeat("offers", || self.offers(&keys)).await?;
        log(&format!("setup complete in {:.0}s", t0.elapsed().as_secs_f64()));
        Ok(())
    }

    /// Run a phase until it submits nothing.
    async fn repeat<F, Fut>(&self, name: &str, mut phase: F) -> Result<(), String>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<Item>, String>>,
    {
        for round in 1..=MAX_ROUNDS {
            let t = Instant::now();
            let items = phase().await?;
            if items.is_empty() {
                log(&format!("{name}: done"));
                return Ok(());
            }
            let count = items.len();
            let results = self.submit(items).await;
            let v = node::validated_index(self.pool.any()).await?;
            node::wait_validated(self.pool.any(), v + 3).await?;
            log(&format!(
                "{name}: round {round}, {count} submitted in {:.0}s, results {results:?}",
                t.elapsed().as_secs_f64()
            ));
        }
        Err(format!("{name}: still incomplete after {MAX_ROUNDS} rounds"))
    }

    /// Submit in order at `rate` per second; one account's transactions go
    /// on one connection. Each is signed when sent, with twice the current
    /// open-ledger fee (polled every 200 ms), so it enters the open ledger
    /// instead of the queue.
    async fn submit(&self, items: Vec<Item>) -> BTreeMap<String, u64> {
        let fee = Arc::new(AtomicU64::new(12));
        let last = Arc::new(AtomicU32::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let poller = {
            let (fee, last, stop) = (Arc::clone(&fee), Arc::clone(&last), Arc::clone(&stop));
            let client = Arc::clone(self.pool.any());
            tokio::spawn(async move {
                while !stop.load(Ordering::Relaxed) {
                    if let Ok(f) = node::open_ledger_fee(&client).await {
                        fee.store((f * 2).clamp(12, SETUP_MAX_FEE), Ordering::Relaxed);
                    }
                    if let Ok(v) = node::validated_index(&client).await {
                        last.store(v, Ordering::Relaxed);
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            })
        };
        while last.load(Ordering::Relaxed) == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let start = Instant::now();
        let mut results = BTreeMap::new();
        let mut inflight = FuturesUnordered::new();
        for (i, (account, sequence, build)) in items.into_iter().enumerate() {
            let due = Duration::from_secs_f64(i as f64 / self.rate as f64);
            if let Some(wait) = due.checked_sub(start.elapsed()) {
                tokio::time::sleep(wait).await;
            }
            let c = Common {
                sequence,
                fee_drops: fee.load(Ordering::Relaxed),
                last_ledger: last.load(Ordering::Relaxed) + 20,
            };
            let signed = build(c);
            match self
                .pool
                .route(&account)
                .send(json!({"command": "submit", "tx_blob": signed.blob}))
                .await
            {
                Ok(rx) => inflight.push(rx),
                Err(e) => *results.entry(e).or_insert(0) += 1,
            }
            while inflight.len() > 20_000 {
                if let Some(r) = inflight.next().await {
                    count(&mut results, r);
                }
            }
        }
        while let Some(r) = inflight.next().await {
            count(&mut results, r);
        }
        stop.store(true, Ordering::Relaxed);
        let _ = poller.await;
        results
    }

    async fn sequence(&self, account: &AccountID) -> Result<u32, String> {
        node::account_info(self.pool.route(account), account)
            .await?
            .map(|(s, _)| s)
            .ok_or_else(|| format!("{} does not exist", to_base58(*account)))
    }

    /// Which of `accounts` exist, read with many requests in flight.
    async fn existing(&self, accounts: &[AccountID]) -> Result<HashSet<AccountID>, String> {
        let replies = self
            .query_all_(accounts, |a| json!({"command": "account_info", "account": to_base58(*a), "ledger_index": "validated"}))
            .await?;
        Ok(accounts
            .iter()
            .zip(replies)
            .filter(|(_, r)| r.pointer("/result/account_data").is_some())
            .map(|(a, _)| *a)
            .collect())
    }

    async fn query_all_(
        &self,
        accounts: &[AccountID],
        request: impl Fn(&AccountID) -> Value,
    ) -> Result<Vec<Value>, String> {
        node::query_all(&self.pool, accounts, request).await
    }

    async fn fund_core(&self, k: &Keys) -> Result<Vec<Item>, String> {
        let per_hub = TEST_XRP * u64::from(self.layout.tests.div_ceil(HUBS)) + 100_000_000;
        let mut wanted: Vec<(AccountID, u64)> = vec![(k.issuer.account, ISSUER_XRP), (k.mm.account, MM_XRP)];
        wanted.extend(k.hubs.iter().map(|h| (h.account, per_hub)));
        let accounts: Vec<AccountID> = wanted.iter().map(|w| w.0).collect();
        let have = self.existing(&accounts).await?;
        let mut seq = self.sequence(&self.funder).await?;
        let mut out: Vec<Item> = Vec::new();
        for (account, drops) in wanted.into_iter().filter(|w| !have.contains(&w.0)) {
            let (funder, key) = (self.funder, Arc::clone(&k.funder_key));
            out.push((funder, seq, Box::new(move |c| tx::payment(funder, &key, account, xrp(drops), None, None, c))));
            seq += 1;
        }
        Ok(out)
    }

    async fn fund_tests(&self, k: &Keys) -> Result<Vec<Item>, String> {
        let accounts: Vec<AccountID> = k.tests.iter().map(|t| t.account).collect();
        let have = self.existing(&accounts).await?;
        let mut by_hub: HashMap<usize, Vec<AccountID>> = HashMap::new();
        for (n, a) in accounts.iter().enumerate() {
            if !have.contains(a) {
                by_hub.entry(n % HUBS as usize).or_default().push(*a);
            }
        }
        // Interleave hubs so each ledger gets a few payments from many hubs.
        let mut per_hub: Vec<(Arc<Key>, u32, Vec<AccountID>)> = Vec::new();
        for (hub, missing) in by_hub {
            let h = Arc::clone(&k.hubs[hub]);
            let seq = self.sequence(&h.account).await?;
            per_hub.push((h, seq, missing));
        }
        let mut out: Vec<Item> = Vec::new();
        let longest = per_hub.iter().map(|p| p.2.len()).max().unwrap_or(0);
        for round in 0..longest {
            for (h, seq, missing) in &per_hub {
                if let Some(&a) = missing.get(round) {
                    let h = Arc::clone(h);
                    out.push((h.account, seq + round as u32, Box::new(move |c| tx::payment(h.account, &h, a, xrp(TEST_XRP), None, None, c))));
                }
            }
        }
        Ok(out)
    }

    async fn default_ripple(&self, k: &Keys) -> Result<Vec<Item>, String> {
        let (seq, flags) = node::account_info(self.pool.route(&k.issuer.account), &k.issuer.account)
            .await?
            .ok_or("issuer missing")?;
        if flags & LSF_DEFAULT_RIPPLE != 0 {
            return Ok(Vec::new());
        }
        let issuer = Arc::clone(&k.issuer);
        Ok(vec![(issuer.account, seq, Box::new(move |c| tx::account_set_flag(&issuer, ASF_DEFAULT_RIPPLE, c)))])
    }

    /// Currencies each account already has a trust line for, with balances.
    async fn lines(&self, accounts: &[AccountID]) -> Result<Vec<HashMap<String, f64>>, String> {
        let replies = self
            .query_all_(accounts, |a| json!({"command": "account_lines", "account": to_base58(*a), "ledger_index": "validated", "limit": 400}))
            .await?;
        Ok(replies
            .iter()
            .map(|r| {
                r.pointer("/result/lines")
                    .and_then(Value::as_array)
                    .map(|lines| {
                        lines
                            .iter()
                            .filter_map(|l| {
                                Some((
                                    l.get("currency")?.as_str()?.to_owned(),
                                    l.get("balance")?.as_str()?.parse().ok()?,
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect())
    }

    fn holders(&self, k: &Keys) -> Vec<Arc<Key>> {
        std::iter::once(Arc::clone(&k.mm))
            .chain(k.tests.iter().take(self.layout.iou_group as usize).cloned())
            .collect()
    }

    async fn trust_lines(&self, k: &Keys) -> Result<Vec<Item>, String> {
        let holders = self.holders(k);
        let accounts: Vec<AccountID> = holders.iter().map(|h| h.account).collect();
        let lines = self.lines(&accounts).await?;
        let issuer = k.issuer.account;
        let mut out: Vec<Item> = Vec::new();
        for (holder, have) in holders.into_iter().zip(lines) {
            let missing: Vec<&'static str> = CURRENCIES.iter().copied().filter(|c| !have.contains_key(*c)).collect();
            if missing.is_empty() {
                continue;
            }
            let mut seq = self.sequence(&holder.account).await?;
            for code in missing {
                let holder = Arc::clone(&holder);
                out.push((holder.account, seq, Box::new(move |c| tx::trust_set(&holder, iou(currency(code), issuer, 1, 15), c))));
                seq += 1;
            }
        }
        Ok(out)
    }

    async fn issue(&self, k: &Keys) -> Result<Vec<Item>, String> {
        let holders = self.holders(k);
        let accounts: Vec<AccountID> = holders.iter().map(|h| h.account).collect();
        let lines = self.lines(&accounts).await?;
        let mut seq = self.sequence(&k.issuer.account).await?;
        let mut out: Vec<Item> = Vec::new();
        for (i, (holder, have)) in holders.iter().zip(lines).enumerate() {
            // The market maker gets 1e12 of every currency, the IOU group 1e6 USD.
            let wanted: Vec<(&'static str, i32)> = if i == 0 {
                CURRENCIES.iter().map(|c| (*c, 12)).collect()
            } else {
                vec![("USD", 6)]
            };
            for (code, exp) in wanted {
                if have.get(code).copied().unwrap_or(0.0) > 0.0 || !have.contains_key(code) {
                    continue;
                }
                let (issuer, to) = (Arc::clone(&k.issuer), holder.account);
                out.push((
                    issuer.account,
                    seq,
                    Box::new(move |c| tx::payment(issuer.account, &issuer, to, iou(currency(code), issuer.account, 1, exp), None, None, c)),
                ));
                seq += 1;
            }
        }
        Ok(out)
    }

    /// Every pool: XRP with each currency, and each pair of currencies.
    fn pairs() -> Vec<(Option<usize>, usize)> {
        let n = CURRENCIES.len();
        let mut p: Vec<(Option<usize>, usize)> = (0..n).map(|j| (None, j)).collect();
        for i in 0..n {
            for j in i + 1..n {
                p.push((Some(i), j));
            }
        }
        p
    }

    fn asset_json(&self, k: &Keys, side: Option<usize>) -> Value {
        match side {
            None => json!({"currency": "XRP"}),
            Some(i) => json!({"currency": CURRENCIES[i], "issuer": to_base58(k.issuer.account)}),
        }
    }

    async fn amms(&self, k: &Keys) -> Result<Vec<Item>, String> {
        let mut missing = Vec::new();
        for (a, b) in Self::pairs() {
            let r = self
                .pool
                .any()
                .call(json!({"command": "amm_info", "asset": self.asset_json(k, a), "asset2": self.asset_json(k, Some(b)), "ledger_index": "validated"}))
                .await?;
            if r.pointer("/result/amm").is_none() {
                missing.push((a, b));
            }
        }
        let mut seq = self.sequence(&k.mm.account).await?;
        let issuer = k.issuer.account;
        let mut out: Vec<Item> = Vec::new();
        for (a, b) in missing {
            let mm = Arc::clone(&k.mm);
            out.push((
                mm.account,
                seq,
                Box::new(move |c| {
                    let first = match a {
                        None => xrp(POOL_XRP),
                        Some(i) => iou(currency(CURRENCIES[i]), issuer, 1, POOL_IOU_EXP),
                    };
                    let second = iou(currency(CURRENCIES[b]), issuer, 1, POOL_IOU_EXP);
                    // AMMCreate's fee is at least the owner reserve increment.
                    let c = Common { fee_drops: c.fee_drops.max(AMM_CREATE_FEE), ..c };
                    tx::amm_create(&mm, first, second, c)
                }),
            ));
            seq += 1;
        }
        Ok(out)
    }

    /// One standing offer in each direction of every pair, priced 0.1%
    /// worse for the taker than the pools. Better-priced offers would cross
    /// the pools (and each other) as soon as they were placed. Payments still
    /// look up both books and pools in every step.
    async fn offers(&self, k: &Keys) -> Result<Vec<Item>, String> {
        let r = self
            .pool
            .any()
            .call(json!({"command": "account_offers", "account": to_base58(k.mm.account), "ledger_index": "validated", "limit": 400}))
            .await?;
        let code = |v: &Value| -> String {
            v.get("currency").and_then(Value::as_str).map_or_else(|| "XRP".to_owned(), str::to_owned)
        };
        let have: HashSet<(String, String)> = r
            .pointer("/result/offers")
            .and_then(Value::as_array)
            .map(|offers| {
                offers
                    .iter()
                    .map(|o| (code(&o["taker_pays"]), code(&o["taker_gets"])))
                    .collect()
            })
            .unwrap_or_default();
        let name = |side: Option<usize>| side.map_or("XRP", |i| CURRENCIES[i]).to_owned();
        let issuer = k.issuer.account;
        let amount = move |side: Option<usize>, scale: u64| match side {
            None => xrp(OFFER_XRP / 1000 * scale),
            Some(i) => iou(currency(CURRENCIES[i]), issuer, scale, OFFER_IOU_EXP - 3),
        };
        let mut seq = self.sequence(&k.mm.account).await?;
        let mut out: Vec<Item> = Vec::new();
        for (a, b) in Self::pairs() {
            for (pays, gets) in [(a, Some(b)), (Some(b), a)] {
                if have.contains(&(name(pays), name(gets))) {
                    continue;
                }
                let mm = Arc::clone(&k.mm);
                // Taker pays 1001 units for 1000: 0.1% worse than 1:1.
                out.push((mm.account, seq, Box::new(move |c| tx::offer_create(&mm, amount(pays, 1001), amount(gets, 1000), c))));
                seq += 1;
            }
        }
        Ok(out)
    }
}

fn count(results: &mut BTreeMap<String, u64>, r: Result<Value, tokio::sync::oneshot::error::RecvError>) {
    let code = match r {
        Ok(v) => engine_result(&v),
        Err(_) => "connection_closed".into(),
    };
    let key = if accepted(&code) { "accepted".to_owned() } else { code };
    *results.entry(key).or_insert(0) += 1;
}

pub fn log(message: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64());
    eprintln!("{now:.1} {message}");
}
