//! Throughput load generator for a private quaxar network
//! (docs/perf/2026-11-tps-test-plan.md).
//!
//!   tps-loadgen keys  --tag T
//!   tps-loadgen setup --tag T --funder rADDR --nodes ws://a:6006,ws://b:6006
//!                     [--tests 250000] [--iou-group 20000] [--rate 1000] [--conns 4]
//!   tps-loadgen run   --tag T --nodes ws://a:6006 --host 0 --hosts 4
//!                     --steps 100:120,200:120 [--mix xrp=20,iou=20,p1=20,p3=20,p6=20]
//!                     [--start-at UNIX] [--fee-mult 1.2] [--max-fee 1000000]
//!                     [--tests 250000] [--iou-group 20000] [--conns 8] --out run.jsonl
//!   tps-loadgen watch --validators ws://v1:6006,... --seconds S --out ledgers.jsonl
//!                     [--sample-every 15]

mod accounts;
mod node;
mod run;
mod setup;
mod tx;
mod watch;
mod ws;

use std::collections::HashMap;

fn flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let key = k.strip_prefix("--").ok_or(format!("unexpected argument {k}"))?;
        let v = it.next().ok_or(format!("--{key} needs a value"))?;
        m.insert(key.to_owned(), v.clone());
    }
    Ok(m)
}

fn get<T: std::str::FromStr>(m: &HashMap<String, String>, key: &str, default: Option<T>) -> Result<T, String> {
    match m.get(key) {
        Some(v) => v.parse().map_err(|_| format!("--{key}: bad value {v}")),
        None => default.ok_or(format!("--{key} is required")),
    }
}

fn list(m: &HashMap<String, String>, key: &str) -> Result<Vec<String>, String> {
    Ok(get::<String>(m, key, None)?.split(',').map(str::to_owned).collect())
}

fn layout(m: &HashMap<String, String>) -> Result<accounts::Layout, String> {
    let l = accounts::Layout { tests: get(m, "tests", Some(250_000))?, iou_group: get(m, "iou-group", Some(20_000))? };
    if l.iou_group == 0 || l.iou_group >= l.tests {
        return Err("--iou-group must be between 1 and --tests".into());
    }
    Ok(l)
}

async fn main_async(args: Vec<String>) -> Result<(), String> {
    let cmd = args.get(1).ok_or("command required: keys | setup | run | watch")?.clone();
    let m = flags(&args[2..])?;
    match cmd.as_str() {
        "keys" => {
            let tag: String = get(&m, "tag", None)?;
            for (name, index) in [
                ("funder_regular_key", accounts::FUNDER_KEY),
                ("issuer", accounts::ISSUER),
                ("market_maker", accounts::MARKET_MAKER),
                ("first_test", accounts::FIRST_TEST),
            ] {
                println!("{name} {}", protocol::to_base58(accounts::derive(&tag, index).account));
            }
            Ok(())
        }
        "setup" => {
            let pool = node::Pool::connect(&list(&m, "nodes")?, get(&m, "conns", Some(4))?).await?;
            let funder = protocol::parse_base58_account_id(&get::<String>(&m, "funder", None)?).ok_or("bad --funder")?;
            setup::Setup { pool, tag: get(&m, "tag", None)?, funder, layout: layout(&m)?, rate: get(&m, "rate", Some(1000))? }
                .run()
                .await
        }
        "run" => {
            let pool = node::Pool::connect(&list(&m, "nodes")?, get(&m, "conns", Some(8))?).await?;
            let cfg = run::RunConfig {
                tag: get(&m, "tag", None)?,
                layout: layout(&m)?,
                host: get(&m, "host", Some(0))?,
                hosts: get(&m, "hosts", Some(1))?,
                steps: run::parse_steps(&get::<String>(&m, "steps", None)?)?,
                mix: run::parse_mix(&get(&m, "mix", Some("xrp=20,iou=20,p1=20,p3=20,p6=20".to_owned()))?)?,
                start_at: m.get("start-at").map(|v| v.parse()).transpose().map_err(|_| "bad --start-at")?,
                fee_mult: get(&m, "fee-mult", Some(1.2))?,
                max_fee: get(&m, "max-fee", Some(1_000_000))?,
                out: get(&m, "out", None)?,
            };
            if cfg.host >= cfg.hosts {
                return Err("--host must be below --hosts".into());
            }
            run::run(pool, cfg).await
        }
        "watch" => {
            watch::watch(
                &list(&m, "validators")?,
                get(&m, "seconds", None)?,
                &get::<String>(&m, "out", None)?,
                get(&m, "sample-every", Some(15))?,
            )
            .await
        }
        other => Err(format!("unknown command {other}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("runtime");
    if let Err(e) = runtime.block_on(main_async(args)) {
        eprintln!("tps-loadgen: {e}");
        std::process::exit(1);
    }
}
