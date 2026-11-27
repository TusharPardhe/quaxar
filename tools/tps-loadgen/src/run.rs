//! Open-loop payment load from one load host.
//!
//! Each host takes its share of the test accounts, so no account is used by
//! two hosts. Within a host, senders are taken in a cycle, so an account
//! sends at most once per cycle: with 60k accounts per host that is once per
//! several ledgers at any tested rate.
//!
//! Transactions are signed on worker threads ahead of time and sent on a
//! fixed schedule (`steps`, total network rates divided across hosts)
//! whatever the responses are. A rejected transaction marks its sender for
//! a sequence refresh; the sender is skipped until refreshed.

use crate::accounts::{self, CURRENCIES, ISSUER, Key, Layout};
use crate::node::{self, Pool, accepted, engine_result};
use crate::tx::{self, Common, Signed, currency, iou, xrp};
use protocol::{AccountID, STPathSet, to_base58};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Xrp,
    Iou,
    Path1,
    Path3,
    Path6,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Xrp, Kind::Iou, Kind::Path1, Kind::Path3, Kind::Path6];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Xrp => "xrp",
            Kind::Iou => "iou",
            Kind::Path1 => "p1",
            Kind::Path3 => "p3",
            Kind::Path6 => "p6",
        }
    }
}

/// Integer weights per kind, from `xrp=20,iou=20,...`. Missing kinds get 0.
pub fn parse_mix(text: &str) -> Result<[u32; 5], String> {
    let mut w = [0u32; 5];
    for part in text.split(',').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').ok_or(format!("bad mix part {part}"))?;
        let i = Kind::ALL.iter().position(|kind| kind.name() == k).ok_or(format!("unknown kind {k}"))?;
        w[i] = v.parse().map_err(|_| format!("bad weight {v}"))?;
    }
    if w.iter().sum::<u32>() == 0 {
        return Err("mix has no weight".into());
    }
    Ok(w)
}

/// A repeating sequence of kinds with the given weights, spread evenly.
pub fn kind_cycle(weights: [u32; 5]) -> Vec<Kind> {
    let total: u32 = weights.iter().sum();
    let mut credit = [0i64; 5];
    (0..total)
        .map(|_| {
            for i in 0..5 {
                credit[i] += i64::from(weights[i]);
            }
            let best = (0..5).max_by_key(|&i| (credit[i], -(i as i64))).expect("5 kinds");
            credit[best] -= i64::from(total);
            Kind::ALL[best]
        })
        .collect()
}

/// `rate:seconds,...` with total network rates.
pub fn parse_steps(text: &str) -> Result<Vec<(f64, f64)>, String> {
    text.split(',')
        .map(|s| {
            let (r, d) = s.split_once(':').ok_or(format!("bad step {s}"))?;
            Ok((r.parse().map_err(|_| format!("bad rate {r}"))?, d.parse().map_err(|_| format!("bad duration {d}"))?))
        })
        .collect()
}

/// Range of `0..len` owned by host `host` of `hosts`.
pub fn share(len: u32, host: u32, hosts: u32) -> std::ops::Range<u32> {
    let lo = (u64::from(len) * u64::from(host) / u64::from(hosts)) as u32;
    let hi = (u64::from(len) * u64::from(host + 1) / u64::from(hosts)) as u32;
    lo..hi
}

pub struct RunConfig {
    pub tag: String,
    pub layout: Layout,
    pub host: u32,
    pub hosts: u32,
    pub steps: Vec<(f64, f64)>,
    pub mix: [u32; 5],
    pub start_at: Option<f64>,
    pub fee_mult: f64,
    pub max_fee: u64,
    pub out: String,
}

struct Sender {
    key: Key,
    seq: AtomicU32,
    dirty: AtomicBool,
}

struct Item {
    sender: usize,
    account: AccountID,
    kind: Kind,
    signed: Signed,
}

#[derive(Default)]
struct Second {
    sent: u64,
    by_kind: [u64; 5],
    codes: BTreeMap<String, u64>,
    latencies_us: Vec<u32>,
}

pub async fn run(pool: Pool, cfg: RunConfig) -> Result<(), String> {
    let pool = Arc::new(pool);
    let general = share(cfg.layout.tests - cfg.layout.iou_group, cfg.host, cfg.hosts);
    let general = (general.start + cfg.layout.iou_group)..(general.end + cfg.layout.iou_group);
    let iou_share = share(cfg.layout.iou_group, cfg.host, cfg.hosts);
    crate::setup::log(&format!(
        "host {}/{}: general accounts {:?}, IOU accounts {:?}",
        cfg.host, cfg.hosts, general, iou_share
    ));
    let first = accounts::FIRST_TEST;
    let to_index = |r: &std::ops::Range<u32>| (first + r.start)..(first + r.end);
    let general_keys = accounts::derive_range(&cfg.tag, to_index(&general));
    let iou_keys = accounts::derive_range(&cfg.tag, to_index(&iou_share));
    let issuer = accounts::derive(&cfg.tag, ISSUER).account;

    // Senders: general accounts, then the IOU share. Receivers are drawn from
    // the same two lists at an offset of half a cycle.
    let n_general = general_keys.len();
    let senders: Arc<Vec<Sender>> = Arc::new(
        general_keys
            .into_iter()
            .chain(iou_keys)
            .map(|key| Sender { key, seq: AtomicU32::new(0), dirty: AtomicBool::new(true) })
            .collect(),
    );
    refresh(&pool, &senders, (0..senders.len()).collect()).await?;
    let missing = senders.iter().filter(|s| s.dirty.load(Ordering::Relaxed)).count();
    crate::setup::log(&format!("sequences loaded, {missing} accounts missing"));

    let fee = Arc::new(AtomicU64::new(12));
    let last_validated = Arc::new(AtomicU32::new(node::validated_index(pool.any()).await?));
    spawn_pollers(Arc::clone(&pool), Arc::clone(&fee), Arc::clone(&last_validated), cfg.fee_mult, cfg.max_fee);
    spawn_refresher(Arc::clone(&pool), Arc::clone(&senders));

    if let Some(at) = cfg.start_at {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        if at > now {
            tokio::time::sleep(Duration::from_secs_f64(at - now)).await;
        }
    }

    // Producer: specs in a cycle, signed on worker threads.
    let total_secs: f64 = cfg.steps.iter().map(|s| s.1).sum();
    let peak: f64 = cfg.steps.iter().map(|s| s.0).fold(0.0, f64::max) / f64::from(cfg.hosts);
    // About a quarter second of signed transactions in hand: each is signed
    // with the fee and last ledger current when it is signed.
    let (blob_tx, mut blob_rx) = tokio::sync::mpsc::channel::<Item>((peak as usize / 4).max(32));
    let stop = Arc::new(AtomicBool::new(false));
    let producer = {
        let senders = Arc::clone(&senders);
        let fee = Arc::clone(&fee);
        let last_validated = Arc::clone(&last_validated);
        let stop = Arc::clone(&stop);
        let cycle = kind_cycle(cfg.mix);
        std::thread::spawn(move || produce(senders, n_general, issuer, cycle, fee, last_validated, blob_tx, stop))
    };

    let stats: Arc<Mutex<BTreeMap<u64, Second>>> = Arc::default();
    let start = Instant::now();
    let mut sent_total = 0u64;
    let mut due_total = 0.0f64;
    let mut step_start = 0.0f64;
    let mut out = std::fs::File::create(&cfg.out).map_err(|e| format!("{}: {e}", cfg.out))?;
    let mut flushed_to = 0u64;
    let mut ticker = tokio::time::interval(Duration::from_millis(2));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut step = 0usize;
    loop {
        ticker.tick().await;
        let t = start.elapsed().as_secs_f64();
        while step < cfg.steps.len() && t >= step_start + cfg.steps[step].1 {
            step_start += cfg.steps[step].1;
            step += 1;
        }
        if step >= cfg.steps.len() {
            break;
        }
        // Due count integrates the per-host rate of each step.
        let rate = cfg.steps[step].0 / f64::from(cfg.hosts);
        let prior: f64 = cfg.steps[..step].iter().map(|s| s.0 / f64::from(cfg.hosts) * s.1).sum();
        due_total = prior + rate * (t - step_start);
        while (sent_total as f64) < due_total {
            let Some(item) = blob_rx.recv().await else { break };
            let second = start.elapsed().as_secs();
            {
                let mut s = stats.lock().expect("stats");
                let e = s.entry(second).or_default();
                e.sent += 1;
                e.by_kind[item.kind as usize] += 1;
            }
            let client = Arc::clone(pool.route(&item.account));
            let senders = Arc::clone(&senders);
            let stats = Arc::clone(&stats);
            let sent_at = Instant::now();
            match client.send(json!({"command": "submit", "tx_blob": item.signed.blob})).await {
                Ok(rx) => {
                    tokio::spawn(async move {
                        let code = match rx.await {
                            Ok(v) => engine_result(&v),
                            Err(_) => "connection_closed".into(),
                        };
                        if !accepted(&code) {
                            senders[item.sender].dirty.store(true, Ordering::Relaxed);
                        }
                        let us = sent_at.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
                        let mut s = stats.lock().expect("stats");
                        let e = s.entry(second).or_default();
                        *e.codes.entry(code).or_insert(0) += 1;
                        e.latencies_us.push(us);
                    });
                }
                Err(e) => {
                    let mut s = stats.lock().expect("stats");
                    *s.entry(second).or_default().codes.entry(e).or_insert(0) += 1;
                }
            }
            sent_total += 1;
        }
        // Write seconds that ended at least 5 s ago (late responses included).
        let now_sec = start.elapsed().as_secs();
        while flushed_to + 5 < now_sec {
            write_second(&mut out, &stats, flushed_to, cfg.steps[step].0, &fee)?;
            flushed_to += 1;
        }
        if t > total_secs {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(5)).await;
    let end_sec = start.elapsed().as_secs();
    while flushed_to <= end_sec {
        write_second(&mut out, &stats, flushed_to, 0.0, &fee)?;
        flushed_to += 1;
    }
    drop(blob_rx);
    let _ = producer.join();
    crate::setup::log(&format!("sent {sent_total} (due {due_total:.0}) in {:.0}s", start.elapsed().as_secs_f64()));
    Ok(())
}

fn write_second(
    out: &mut std::fs::File,
    stats: &Mutex<BTreeMap<u64, Second>>,
    second: u64,
    target: f64,
    fee: &AtomicU64,
) -> Result<(), String> {
    let Some(mut s) = stats.lock().expect("stats").remove(&second) else {
        return Ok(());
    };
    s.latencies_us.sort_unstable();
    let pct = |p: f64| -> u32 {
        if s.latencies_us.is_empty() {
            0
        } else {
            s.latencies_us[((s.latencies_us.len() as f64 * p) as usize).min(s.latencies_us.len() - 1)]
        }
    };
    let by_kind: serde_json::Map<String, Value> =
        Kind::ALL.iter().map(|k| (k.name().to_owned(), Value::from(s.by_kind[*k as usize]))).collect();
    let line = json!({
        "kind": "second",
        "unix": SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()),
        "second": second,
        "target_total": target,
        "sent": s.sent,
        "by_kind": by_kind,
        "codes": s.codes,
        "submit_p50_us": pct(0.5),
        "submit_p90_us": pct(0.9),
        "submit_p99_us": pct(0.99),
        "fee_drops": fee.load(Ordering::Relaxed),
    });
    writeln!(out, "{line}").map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn produce(
    senders: Arc<Vec<Sender>>,
    n_general: usize,
    issuer: AccountID,
    cycle: Vec<Kind>,
    fee: Arc<AtomicU64>,
    last_validated: Arc<AtomicU32>,
    out: tokio::sync::mpsc::Sender<Item>,
    stop: Arc<AtomicBool>,
) {
    let n_iou = senders.len() - n_general;
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).max(1);
    let paths: Arc<Vec<[STPathSet; 3]>> = Arc::new(
        (0..CURRENCIES.len())
            .map(|d| [tx::path_set(issuer, d, 1, 1), tx::path_set(issuer, d, 3, 3), tx::path_set(issuer, d, 6, 8)])
            .collect(),
    );
    let currencies: Arc<Vec<protocol::Currency>> = Arc::new(CURRENCIES.iter().map(|c| currency(c)).collect());
    let (spec_tx, spec_rx) = std::sync::mpsc::sync_channel::<(usize, usize, Kind, usize, u32)>(workers * 4);
    let spec_rx = Arc::new(Mutex::new(spec_rx));
    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let spec_rx = Arc::clone(&spec_rx);
            let senders = Arc::clone(&senders);
            let out = out.clone();
            let fee = Arc::clone(&fee);
            let last_validated = Arc::clone(&last_validated);
            let paths = Arc::clone(&paths);
            let currencies = Arc::clone(&currencies);
            std::thread::spawn(move || {
                loop {
                    let next = spec_rx.lock().expect("spec rx").recv();
                    let Ok((s, r, kind, dest, sequence)) = next else { break };
                    let c = Common {
                        sequence,
                        fee_drops: fee.load(Ordering::Relaxed),
                        last_ledger: last_validated.load(Ordering::Relaxed) + 10,
                    };
                    let key = &senders[s].key;
                    let to = senders[r].key.account;
                    let signed = match kind {
                        Kind::Xrp => tx::payment(key.account, key, to, xrp(1000), None, None, c),
                        Kind::Iou => tx::payment(key.account, key, to, iou(currencies[0], issuer, 1, -2), None, None, c),
                        Kind::Path1 | Kind::Path3 | Kind::Path6 => {
                            let set = match kind {
                                Kind::Path1 => &paths[dest][0],
                                Kind::Path3 => &paths[dest][1],
                                _ => &paths[dest][2],
                            };
                            tx::payment(
                                key.account,
                                key,
                                to,
                                iou(currencies[dest], issuer, 1, -2),
                                Some(xrp(1_000_000)),
                                Some(set.clone()),
                                c,
                            )
                        }
                    };
                    let item = Item { sender: s, account: key.account, kind, signed };
                    if out.blocking_send(item).is_err() {
                        break;
                    }
                }
            })
        })
        .collect();
    // Only the workers may hold the receiver. If this function kept it, the
    // workers' exit (when `run` drops the blob channel) would leave `send`
    // below blocked on a full channel forever, and `run` hung in `join`.
    drop(spec_rx);

    let (mut g, mut i, mut k, mut d) = (0usize, 0usize, 0usize, 0usize);
    while !stop.load(Ordering::Relaxed) {
        let kind = cycle[k % cycle.len()];
        k += 1;
        // Sender and receiver for this kind; skip senders awaiting a refresh.
        let (sender, receiver) = match kind {
            Kind::Iou if n_iou > 1 => {
                let s = n_general + i % n_iou;
                let r = n_general + (i + n_iou / 2) % n_iou;
                i += 1;
                (s, r)
            }
            Kind::Iou => continue,
            Kind::Xrp => {
                let s = g % n_general;
                let r = (g + n_general / 2) % n_general;
                g += 1;
                (s, r)
            }
            _ if n_iou > 0 => {
                let s = g % n_general;
                let r = n_general + (g + n_iou / 3) % n_iou;
                g += 1;
                (s, r)
            }
            _ => continue,
        };
        if sender == receiver || senders[sender].dirty.load(Ordering::Relaxed) {
            continue;
        }
        let seq = senders[sender].seq.fetch_add(1, Ordering::Relaxed);
        d = (d + 1) % CURRENCIES.len();
        if spec_tx.send((sender, receiver, kind, d, seq)).is_err() {
            break;
        }
    }
    drop(spec_tx);
    for h in handles {
        let _ = h.join();
    }
}

/// Read the next sequence of each sender in `indices`; clears `dirty` for
/// accounts that exist.
async fn refresh(pool: &Pool, senders: &[Sender], indices: Vec<usize>) -> Result<(), String> {
    let accounts: Vec<AccountID> = indices.iter().map(|&i| senders[i].key.account).collect();
    let replies = node::query_all(pool, &accounts, |a| {
        json!({"command": "account_info", "account": to_base58(*a), "ledger_index": "current"})
    })
    .await?;
    for (&i, r) in indices.iter().zip(replies) {
        if let Some(seq) = r.pointer("/result/account_data/Sequence").and_then(Value::as_u64) {
            senders[i].seq.store(seq as u32, Ordering::Relaxed);
            senders[i].dirty.store(false, Ordering::Relaxed);
        }
    }
    Ok(())
}

fn spawn_refresher(pool: Arc<Pool>, senders: Arc<Vec<Sender>>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let dirty: Vec<usize> = (0..senders.len()).filter(|&i| senders[i].dirty.load(Ordering::Relaxed)).collect();
            if !dirty.is_empty() {
                let _ = refresh(&pool, &senders, dirty).await;
            }
        }
    });
}

fn spawn_pollers(pool: Arc<Pool>, fee: Arc<AtomicU64>, last_validated: Arc<AtomicU32>, mult: f64, max_fee: u64) {
    tokio::spawn(async move {
        loop {
            if let Ok(open) = node::open_ledger_fee(pool.any()).await {
                let f = ((open as f64) * mult).ceil() as u64;
                fee.store(f.clamp(12, max_fee), Ordering::Relaxed);
            }
            if let Ok(v) = node::validated_index(pool.any()).await {
                last_validated.store(v, Ordering::Relaxed);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_cycle_follows_weights() {
        let c = kind_cycle([20, 20, 20, 20, 20]);
        assert_eq!(c.len(), 100);
        for k in Kind::ALL {
            assert_eq!(c.iter().filter(|x| **x == k).count(), 20);
        }
        // Evenly spread: the first five are all different.
        let first: std::collections::HashSet<_> = c[..5].iter().map(|k| k.name()).collect();
        assert_eq!(first.len(), 5);
        let only_xrp = kind_cycle([1, 0, 0, 0, 0]);
        assert_eq!(only_xrp, vec![Kind::Xrp]);
    }

    #[test]
    fn shares_cover_the_range_without_overlap() {
        let parts: Vec<_> = (0..4).map(|h| share(250_000, h, 4)).collect();
        assert_eq!(parts[0].start, 0);
        assert_eq!(parts[3].end, 250_000);
        for w in parts.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
    }

    #[test]
    fn mix_and_steps_parse() {
        assert_eq!(parse_mix("xrp=20,iou=20,p1=20,p3=20,p6=20").unwrap(), [20; 5]);
        assert_eq!(parse_mix("xrp=1").unwrap(), [1, 0, 0, 0, 0]);
        assert!(parse_mix("bad=1").is_err());
        assert_eq!(parse_steps("100:60,200:30").unwrap(), vec![(100.0, 60.0), (200.0, 30.0)]);
    }
}
