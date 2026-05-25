//! Relational database for ledger header persistence.
//!
//!
//! Schema mirrors the reference Ledgers table exactly:
//!   LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime,
//!   CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash
//!
//! On startup, `get_newest_ledger_info()` returns the most recent persisted
//! header so the node can reconstruct the validated ledger from NuDB without
//! re-acquiring it from peers — matching reference `getLastFullLedger()`.

use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

pub use protocol::LedgerHeader;

/// Open (or create) the ledger SQLite database at `path`.
pub struct LedgerDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerFinderBootcacheEntry {
    pub address: String,
    pub valence: i32,
}

/// Open (or create) the PeerFinder SQLite database.
///
/// stores bootcache rows in `PeerFinder_BootstrapCache`.
pub struct PeerFinderDb {
    conn: Mutex<Connection>,
}

impl PeerFinderDb {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS PeerFinder_BootstrapCache (
                 id       INTEGER PRIMARY KEY AUTOINCREMENT,
                 address  TEXT UNIQUE NOT NULL,
                 valence  INTEGER
             );
             CREATE INDEX IF NOT EXISTS PeerFinder_BootstrapCache_Index
                 ON PeerFinder_BootstrapCache (address);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn load_bootcache(&self) -> rusqlite::Result<Vec<PeerFinderBootcacheEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut statement =
            conn.prepare("SELECT address, valence FROM PeerFinder_BootstrapCache;")?;
        let rows = statement.query_map([], |row| {
            Ok(PeerFinderBootcacheEntry {
                address: row.get(0)?,
                valence: row.get(1)?,
            })
        })?;

        rows.collect()
    }

    pub fn save_bootcache(&self, entries: &[PeerFinderBootcacheEntry]) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM PeerFinder_BootstrapCache;", [])?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO PeerFinder_BootstrapCache (address, valence)
                 VALUES (?1, ?2);",
            )?;
            for entry in entries {
                insert.execute(params![entry.address, entry.valence])?;
            }
        }
        tx.commit()
    }
}

impl LedgerDb {
    /// Open or create the database. Creates the Ledgers table if missing.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS Ledgers (
                 LedgerHash      TEXT NOT NULL,
                 LedgerSeq       INTEGER NOT NULL,
                 PrevHash        TEXT NOT NULL,
                 TotalCoins      TEXT NOT NULL,
                 ClosingTime     INTEGER NOT NULL,
                 PrevClosingTime INTEGER NOT NULL,
                 CloseTimeRes    INTEGER NOT NULL,
                 CloseFlags      INTEGER NOT NULL,
                 AccountSetHash  TEXT NOT NULL,
                 TransSetHash    TEXT NOT NULL,
                 PRIMARY KEY (LedgerHash)
             );
             CREATE INDEX IF NOT EXISTS SeqLedger ON Ledgers (LedgerSeq);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Persist a validated ledger header. Matches reference `kADD_LEDGER` INSERT OR REPLACE.
    pub fn insert_ledger(&self, header: &LedgerHeader) -> rusqlite::Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT OR REPLACE INTO Ledgers
             (LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime, PrevClosingTime,
              CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                hex_hash(header.hash.as_uint256().data()),
                header.seq,
                hex_hash(header.parent_hash.as_uint256().data()),
                header.drops.to_string(),
                header.close_time as i64,
                header.parent_close_time as i64,
                header.close_time_resolution as i64,
                header.close_flags as i64,
                hex_hash(header.account_hash.as_uint256().data()),
                hex_hash(header.tx_hash.as_uint256().data()),
            ],
        )?;
        Ok(())
    }

    /// Return the header with the highest LedgerSeq. Matches reference `getNewestLedgerInfo()`.
    /// Used on startup to reconstruct the last validated ledger without peer acquisition.
    pub fn get_newest_ledger_info(&self) -> rusqlite::Result<Option<StoredLedgerInfo>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime,
                        PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash
                 FROM Ledgers ORDER BY LedgerSeq DESC LIMIT 1",
                [],
                row_to_info,
            )
            .optional()
    }

    /// Return the header for a specific sequence. Matches reference `getLedgerInfoByIndex()`.
    pub fn get_ledger_info_by_seq(&self, seq: u32) -> rusqlite::Result<Option<StoredLedgerInfo>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime,
                        PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash
                 FROM Ledgers WHERE LedgerSeq = ?1",
                params![seq],
                row_to_info,
            )
            .optional()
    }

    /// Return the header for a specific hash. Matches reference `getLedgerInfoByHash()`.
    pub fn get_ledger_info_by_hash(
        &self,
        hash_hex: &str,
    ) -> rusqlite::Result<Option<StoredLedgerInfo>> {
        self.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT LedgerHash, LedgerSeq, PrevHash, TotalCoins, ClosingTime,
                        PrevClosingTime, CloseTimeRes, CloseFlags, AccountSetHash, TransSetHash
                 FROM Ledgers WHERE LedgerHash = ?1",
                params![hash_hex],
                row_to_info,
            )
            .optional()
    }

    /// Delete ledger rows with seq < `before_seq`. Used by online-delete.
    pub fn delete_before_seq(&self, before_seq: u32) -> rusqlite::Result<usize> {
        self.conn.lock().unwrap().execute(
            "DELETE FROM Ledgers WHERE LedgerSeq < ?1",
            params![before_seq],
        )
    }
}

/// All fields stored in the Ledgers table, as raw strings/integers.
/// Callers convert to `LedgerHeader` using `to_header()`.
#[derive(Debug, Clone)]
pub struct StoredLedgerInfo {
    pub ledger_hash: String,
    pub ledger_seq: u32,
    pub prev_hash: String,
    pub total_coins: u64,
    pub closing_time: u32,
    pub prev_closing_time: u32,
    pub close_time_res: u8,
    pub close_flags: u8,
    pub account_set_hash: String,
    pub trans_set_hash: String,
}

impl StoredLedgerInfo {
    /// Convert to a `LedgerHeader`. Returns `None` if any hash fails to parse.
    pub fn to_header(&self) -> Option<LedgerHeader> {
        use basics::base_uint::Uint256;
        use basics::sha_map_hash::SHAMapHash;

        let make_hash = |hex: &str| -> Option<SHAMapHash> {
            Some(SHAMapHash::new(Uint256::from_array(parse_hash(hex)?)))
        };

        Some(LedgerHeader {
            seq: self.ledger_seq,
            hash: make_hash(&self.ledger_hash)?,
            parent_hash: make_hash(&self.prev_hash)?,
            account_hash: make_hash(&self.account_set_hash)?,
            tx_hash: make_hash(&self.trans_set_hash)?,
            drops: self.total_coins,
            close_time: self.closing_time,
            parent_close_time: self.prev_closing_time,
            close_time_resolution: self.close_time_res,
            close_flags: self.close_flags,
            validated: false,
            accepted: false,
        })
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn hex_hash(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02X}", b)).collect()
}

fn parse_hash(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut arr = [0u8; 32];
    for i in 0..32 {
        arr[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(arr)
}

fn row_to_info(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredLedgerInfo> {
    let total_coins_str: String = row.get(3)?;
    let total_coins: u64 = total_coins_str.parse().unwrap_or(0);
    Ok(StoredLedgerInfo {
        ledger_hash: row.get(0)?,
        ledger_seq: row.get::<_, i64>(1)? as u32,
        prev_hash: row.get(2)?,
        total_coins,
        closing_time: row.get::<_, i64>(4)? as u32,
        prev_closing_time: row.get::<_, i64>(5)? as u32,
        close_time_res: row.get::<_, i64>(6)? as u8,
        close_flags: row.get::<_, i64>(7)? as u8,
        account_set_hash: row.get(8)?,
        trans_set_hash: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::base_uint::Uint256;
    use basics::sha_map_hash::SHAMapHash;

    fn make_hash(fill: u8) -> SHAMapHash {
        SHAMapHash::new(Uint256::from_array([fill; 32]))
    }

    fn make_header(seq: u32) -> LedgerHeader {
        LedgerHeader {
            seq,
            hash: make_hash(seq as u8),
            parent_hash: make_hash((seq - 1) as u8),
            account_hash: make_hash(0xAA),
            tx_hash: make_hash(0xBB),
            drops: 99_991_120_000_000_000,
            close_time: 831_000_000 + seq,
            parent_close_time: 831_000_000 + seq - 10,
            close_time_resolution: 10,
            close_flags: 0,
            validated: false,
            accepted: false,
        }
    }

    #[test]
    fn insert_and_retrieve() {
        let db = LedgerDb::open(Path::new(":memory:")).unwrap();
        let h1 = make_header(100);
        let h2 = make_header(200);
        db.insert_ledger(&h1).unwrap();
        db.insert_ledger(&h2).unwrap();

        let newest = db.get_newest_ledger_info().unwrap().unwrap();
        assert_eq!(newest.ledger_seq, 200);

        let by_seq = db.get_ledger_info_by_seq(100).unwrap().unwrap();
        assert_eq!(by_seq.ledger_seq, 100);

        let hdr = by_seq.to_header().unwrap();
        assert_eq!(hdr.seq, 100);
        assert_eq!(hdr.close_time, h1.close_time);
        assert_eq!(hdr.drops, h1.drops);
    }

    #[test]
    fn idempotent_insert() {
        let db = LedgerDb::open(Path::new(":memory:")).unwrap();
        let h = make_header(1);
        db.insert_ledger(&h).unwrap();
        db.insert_ledger(&h).unwrap(); // INSERT OR REPLACE — no error
        assert_eq!(db.get_newest_ledger_info().unwrap().unwrap().ledger_seq, 1);
    }

    #[test]
    fn peerfinder_bootcache_round_trips_cpp_schema() {
        let db = PeerFinderDb::open(Path::new(":memory:")).unwrap();
        db.save_bootcache(&[
            PeerFinderBootcacheEntry {
                address: "10.0.0.1:51235".to_owned(),
                valence: 2,
            },
            PeerFinderBootcacheEntry {
                address: "10.0.0.2:51235".to_owned(),
                valence: -1,
            },
        ])
        .unwrap();

        assert_eq!(
            db.load_bootcache().unwrap(),
            vec![
                PeerFinderBootcacheEntry {
                    address: "10.0.0.1:51235".to_owned(),
                    valence: 2,
                },
                PeerFinderBootcacheEntry {
                    address: "10.0.0.2:51235".to_owned(),
                    valence: -1,
                },
            ]
        );
    }
}
