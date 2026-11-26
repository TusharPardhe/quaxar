//! Fork the newest ledger of a stopped quaxar node into the start of a private
//! test network.
//!
//!   tps-fork <fjall-dir> <db-dir> <account> <regular-key>
//!
//! The newest `Ledgers` row in `<db-dir>/ledger.db` names ledger L. The tool
//! rewrites L in place so that `<account>`'s AccountRoot carries
//! `RegularKey = <regular-key>`, a key the test controls. Nothing else
//! changes: no XRP is created or moved, and the sequence, parent, close times
//! and transaction tree stay the same. It then:
//!
//! - writes the new leaf, the inner nodes above it and the new header object
//!   into the `nodes` keyspace, in the same value layout as quaxar;
//! - points L's rows in `ledger.db` and `ledger_headers.db` at the new hash
//!   and state root.
//!
//! The pruning bookkeeping is left as it is. The next claim diffs against the
//! old claimed root, so the new path is counted as new and the old path dies,
//! as for any ledger change.
//!
//! `quaxar --load` then starts from the forked L. Run it only on a copy meant
//! for a private network: the result is not a mainnet ledger.

use basics::base_uint::Uint256;
use fjall::{Database, Keyspace, KeyspaceCreateOptions, PersistMode};
use protocol::{
    LedgerHeader, STObject, SerialIter, calculate_ledger_hash, get_field_by_symbol,
    parse_base58_account_id, serialize_prefixed_ledger_header, sha512_half, to_base58,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

/// Stored value layout (`EncodedBlob`): 8 unused bytes, the type byte, then
/// the node in prefix format.
const TYPE_OFFSET: usize = 8;
const DATA_OFFSET: usize = 9;
const TYPE_LEDGER: u8 = 1;
const INNER_PREFIX: &[u8; 4] = b"MIN\0";
const LEAF_PREFIX: &[u8; 4] = b"MLN\0";
const INNER_LEN: usize = 4 + 16 * 32;

type Hash = [u8; 32];

struct Row {
    header: LedgerHeader,
    /// The hash text as stored, so the update matches its case.
    hash_text: String,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        eprintln!("usage: tps-fork <fjall-dir> <db-dir> <account> <regular-key>");
        std::process::exit(2);
    }
    if let Err(error) = run(Path::new(&args[1]), Path::new(&args[2]), &args[3], &args[4]) {
        eprintln!("tps-fork: {error}");
        std::process::exit(1);
    }
}

fn run(fjall_dir: &Path, db_dir: &Path, account: &str, regular_key: &str) -> Result<(), String> {
    let account = parse_base58_account_id(account).ok_or("account is not a valid address")?;
    let regular_key =
        parse_base58_account_id(regular_key).ok_or("regular key is not a valid address")?;

    let ledger_db = Connection::open(db_dir.join("ledger.db")).map_err(|e| e.to_string())?;
    let row = newest_row(&ledger_db)?;
    let old = row.header;
    if calculate_ledger_hash(&old) != old.hash {
        return Err(format!("ledger {} row hash does not match its header", old.seq));
    }
    println!("ledger {} {} state {}", old.seq, old.hash, old.account_hash);

    let db = Database::builder(fjall_dir)
        .worker_threads(1)
        .open()
        .map_err(|e| format!("open {}: {e}", fjall_dir.display()))?;
    let nodes = db
        .keyspace("nodes", KeyspaceCreateOptions::default)
        .map_err(|e| e.to_string())?;

    let key = account_key(account);
    let (path, leaf_value) = find_leaf(&nodes, hash_bytes(&old.account_hash), &key)?;
    let leaf = &leaf_value[DATA_OFFSET..];
    let sle_bytes = &leaf[4..leaf.len() - 32];
    let new_sle = set_regular_key(sle_bytes, regular_key)?;

    // New leaf, then each inner node up to the root with the new child hash.
    let mut writes: Vec<(Hash, Vec<u8>)> = Vec::new();
    let mut node_data = [LEAF_PREFIX.as_slice(), &new_sle, &key].concat();
    let mut child = sha512_half(&node_data).data().to_owned();
    writes.push((child, with_prefix(&leaf_value, &node_data)));
    for (inner_value, branch) in path.iter().rev() {
        let mut inner = inner_value[DATA_OFFSET..].to_vec();
        let at = 4 + branch * 32;
        inner[at..at + 32].copy_from_slice(&child);
        node_data = inner;
        child = sha512_half(&node_data).data().to_owned();
        writes.push((child, with_prefix(inner_value, &node_data)));
    }

    let mut new = old;
    new.account_hash = basics::sha_map_hash::SHAMapHash::new(Uint256::from_array(child));
    new.hash = calculate_ledger_hash(&new);
    let header_value = {
        let mut v = vec![0u8; DATA_OFFSET];
        v[TYPE_OFFSET] = TYPE_LEDGER;
        v.extend(serialize_prefixed_ledger_header(&new, false));
        v
    };
    writes.push((hash_bytes(&new.hash), header_value));

    for (hash, value) in &writes {
        nodes.insert(hash, value).map_err(|e| e.to_string())?;
    }
    db.persist(PersistMode::SyncAll).map_err(|e| e.to_string())?;

    // Re-read the new path before touching SQLite.
    let (_, check) = find_leaf(&nodes, hash_bytes(&new.account_hash), &key)?;
    if check[DATA_OFFSET..] != *node_bytes_of_leaf(&new_sle, &key) {
        return Err("new leaf does not read back".into());
    }

    update_row(&ledger_db, &row, &new)?;
    let headers_path = db_dir.join("ledger_headers.db");
    if headers_path.exists() {
        let headers = Connection::open(&headers_path).map_err(|e| e.to_string())?;
        if let Some(headers_row) = row_at(&headers, old.seq)? {
            update_row(&headers, &headers_row, &new)?;
        }
    }

    println!(
        "forked ledger {}: {} state {} ({} nodes written)",
        new.seq,
        new.hash,
        new.account_hash,
        writes.len()
    );
    println!("{} now has RegularKey {}", to_base58(account), to_base58(regular_key));
    Ok(())
}

fn account_key(account: protocol::AccountID) -> Hash {
    let id = basics::base_uint::Uint160::from_slice(account.data()).expect("20 bytes");
    protocol::account_root_key(id).data().to_owned()
}

fn hash_bytes(hash: &basics::sha_map_hash::SHAMapHash) -> Hash {
    hash.as_uint256().data().to_owned()
}

fn with_prefix(template: &[u8], data: &[u8]) -> Vec<u8> {
    let mut v = template[..DATA_OFFSET].to_vec();
    v.extend_from_slice(data);
    v
}

fn node_bytes_of_leaf(sle: &[u8], key: &Hash) -> Vec<u8> {
    [LEAF_PREFIX.as_slice(), sle, key].concat()
}

fn nibble(key: &Hash, depth: usize) -> usize {
    let byte = key[depth / 2];
    usize::from(if depth % 2 == 0 { byte >> 4 } else { byte & 0x0F })
}

/// Descend from `root` to the leaf for `key`. Returns the inner nodes on the
/// way (stored value, branch taken) and the leaf's stored value.
fn find_leaf(
    nodes: &Keyspace,
    root: Hash,
    key: &Hash,
) -> Result<(Vec<(Vec<u8>, usize)>, Vec<u8>), String> {
    let mut path = Vec::new();
    let mut at = root;
    for depth in 0..64 {
        let value = nodes
            .get(at)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("node {} missing at depth {depth}", Uint256::from_array(at)))?
            .to_vec();
        let data = value.get(DATA_OFFSET..).ok_or("short node")?;
        if data.starts_with(INNER_PREFIX) {
            if data.len() != INNER_LEN {
                return Err(format!("inner node at depth {depth} has length {}", data.len()));
            }
            let branch = nibble(key, depth);
            let child: Hash = data[4 + branch * 32..4 + branch * 32 + 32]
                .try_into()
                .expect("32 bytes");
            if child == [0u8; 32] {
                return Err("account not found in the state tree".into());
            }
            path.push((value, branch));
            at = child;
        } else if data.starts_with(LEAF_PREFIX) {
            if data.len() < 4 + 32 || data[data.len() - 32..] != key[..] {
                return Err("account not found in the state tree".into());
            }
            return Ok((path, value));
        } else {
            return Err(format!("unexpected node type at depth {depth}"));
        }
    }
    Err("state tree deeper than 64".into())
}

/// Set `RegularKey` on the AccountRoot in `sle` and serialize it. The
/// unmodified entry must serialize back to the same bytes, so the codec is
/// known to be lossless for this entry before it is changed.
fn set_regular_key(sle: &[u8], regular_key: protocol::AccountID) -> Result<Vec<u8>, String> {
    // Parsed without the AccountRoot template: applying the template would
    // add its default fields and change the stored entry.
    let mut entry = STObject::new(get_field_by_symbol("sfLedgerEntry"));
    let mut it = SerialIter::new(sle);
    entry.set_from_serial_iter(&mut it, 0);
    if it.get_bytes_left() != 0 {
        return Err("AccountRoot has trailing bytes".into());
    }
    if entry.get_field_u16(get_field_by_symbol("sfLedgerEntryType")) != 0x0061 {
        return Err("entry is not an AccountRoot".into());
    }
    let again = entry.get_serializer().data().to_vec();
    if again != sle {
        return Err(format!(
            "AccountRoot does not round-trip through the codec:\n stored {}\n again  {}",
            hex(sle),
            hex(&again)
        ));
    }
    let balance = entry.get_field_amount(get_field_by_symbol("sfBalance"));
    println!("account balance {} drops", balance.mantissa());
    entry.set_account_id(get_field_by_symbol("sfRegularKey"), regular_key);
    Ok(entry.get_serializer().data().to_vec())
}

fn parse_hash(text: &str) -> Result<basics::sha_map_hash::SHAMapHash, String> {
    Uint256::from_hex(text)
        .map(basics::sha_map_hash::SHAMapHash::new)
        .map_err(|_| format!("bad hash {text}"))
}

const SELECT: &str = "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, \
     PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash FROM Ledgers";

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, u32, String, String, u32, u32, u32, u32, String, String)> {
    // TotalCoins is BIGINT in ledger.db and TEXT in ledger_headers.db.
    let coins = match row.get_ref(3)? {
        rusqlite::types::ValueRef::Integer(v) => v.to_string(),
        rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        _ => String::new(),
    };
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        coins,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

fn to_row(r: (String, u32, String, String, u32, u32, u32, u32, String, String)) -> Result<Row, String> {
    let header = LedgerHeader {
        hash: parse_hash(&r.0)?,
        seq: r.1,
        parent_hash: parse_hash(&r.2)?,
        drops: r.3.parse().map_err(|_| format!("bad TotalCoins {}", r.3))?,
        close_time: r.4,
        parent_close_time: r.5,
        close_time_resolution: u8::try_from(r.6).map_err(|_| "bad CloseTimeRes")?,
        close_flags: u8::try_from(r.7).map_err(|_| "bad CloseFlags")?,
        account_hash: parse_hash(&r.8)?,
        tx_hash: parse_hash(&r.9)?,
        ..LedgerHeader::default()
    };
    Ok(Row { header, hash_text: r.0 })
}

fn newest_row(conn: &Connection) -> Result<Row, String> {
    let r = conn
        .query_row(&format!("{SELECT} ORDER BY LedgerSeq DESC LIMIT 1"), [], read_row)
        .map_err(|e| format!("newest ledger row: {e}"))?;
    to_row(r)
}

fn row_at(conn: &Connection, seq: u32) -> Result<Option<Row>, String> {
    conn.query_row(&format!("{SELECT} WHERE LedgerSeq = ?1"), [seq], read_row)
        .optional()
        .map_err(|e| e.to_string())?
        .map(to_row)
        .transpose()
}

/// Same text case as the stored hash.
fn format_like(sample: &str, hash: &basics::sha_map_hash::SHAMapHash) -> String {
    let upper = hash.as_uint256().to_string().to_uppercase();
    if sample.chars().any(|c| c.is_ascii_lowercase()) {
        upper.to_lowercase()
    } else {
        upper
    }
}

fn update_row(conn: &Connection, row: &Row, new: &LedgerHeader) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE Ledgers SET LedgerHash = ?1, AccountSetHash = ?2 WHERE LedgerSeq = ?3 AND LedgerHash = ?4",
            params![
                format_like(&row.hash_text, &new.hash),
                format_like(&row.hash_text, &new.account_hash),
                new.seq,
                row.hash_text
            ],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err(format!("expected to update 1 row, updated {changed}"));
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}
