//! Restore watch for the pruned node store (design rules R3/R5).
//!
//! The pruned index watches every node it may delete (dead-pending state
//! nodes and owned nodes). A node that is *written* or *read* through the
//! node store while watched is still needed by someone: a consensus build or
//! an acquisition that will attach it to a ledger that is not yet claimed.
//! Prune must keep such a node instead of deleting it out from under that
//! ledger. Before this watch covered reads, an acquisition could read a dead
//! node, attach it, and then lose it to the next prune: the acquired ledger
//! was complete in memory but had a hole on disk.
//!
//! Prune deletes in three steps so a read or write that races the delete can
//! never be lost:
//!
//! ```text
//!  begin_delete  Watching -> Deleting   (already Restored/Rescued: keep)
//!  commit        the delete batch lands
//!  finish_delete Rescued(bytes) -> re-put by prune; entry removed
//! ```
//!
//! A read of a watched node holds the watch lock across the store read, and
//! a write marks the watch before the store write, so every interleaving with
//! `finish_delete` either sees the node deleted (a plain miss) or leaves the
//! bytes in `Rescued` for prune to write back.

use std::collections::HashMap;
use std::sync::RwLock;

use basics::base_uint::Uint256;

#[derive(Debug, Clone, PartialEq, Eq)]
enum WatchState {
    /// Dead-pending; untouched since the watch started.
    Watching,
    /// Read or written again: prune must keep it.
    Restored,
    /// Prune is deleting it now.
    Deleting,
    /// Read or written while being deleted; prune writes these bytes back.
    Rescued(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct RestoreWatch {
    map: RwLock<HashMap<Uint256, WatchState>>,
}

impl RestoreWatch {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_watched(&self, hash: &Uint256) -> bool {
        let map = self.map.read().expect("restore watch lock");
        !map.is_empty() && map.contains_key(hash)
    }

    pub fn watch(&self, hashes: &[Uint256]) {
        let mut map = self.map.write().expect("restore watch lock");
        for hash in hashes {
            map.entry(*hash).or_insert(WatchState::Watching);
        }
    }

    pub fn unwatch(&self, hashes: &[Uint256]) {
        let mut map = self.map.write().expect("restore watch lock");
        for hash in hashes {
            map.remove(hash);
        }
    }

    pub fn was_restored(&self, hash: &Uint256) -> bool {
        matches!(
            self.map.read().expect("restore watch lock").get(hash),
            Some(WatchState::Restored | WatchState::Rescued(_))
        )
    }

    /// Mark `hash` as needed before its bytes are written to the store.
    pub fn before_write(&self, hash: &Uint256, bytes: impl FnOnce() -> Vec<u8>) {
        if !self.is_watched(hash) {
            return;
        }
        let mut map = self.map.write().expect("restore watch lock");
        if let Some(state) = map.get_mut(hash) {
            *state = match state {
                WatchState::Deleting | WatchState::Rescued(_) => WatchState::Rescued(bytes()),
                _ => WatchState::Restored,
            };
        }
    }

    /// Run a store read; if the node is watched, the read and the state
    /// transition happen under the watch lock so `finish_delete` cannot slip
    /// between them. `read` returns the stored bytes on a hit.
    pub fn read<T>(
        &self,
        hash: &Uint256,
        read: impl FnOnce() -> (T, Option<Vec<u8>>),
    ) -> T {
        if !self.is_watched(hash) {
            return read().0;
        }
        let mut map = self.map.write().expect("restore watch lock");
        let (result, bytes) = read();
        if let (Some(state), Some(bytes)) = (map.get_mut(hash), bytes) {
            *state = match state {
                WatchState::Deleting | WatchState::Rescued(_) => WatchState::Rescued(bytes),
                _ => WatchState::Restored,
            };
        }
        result
    }

    /// Start deleting `hashes`. Returns, per hash, whether it must be kept
    /// because it was read or written since it was watched.
    pub fn begin_delete(&self, hashes: &[Uint256]) -> Vec<bool> {
        let mut map = self.map.write().expect("restore watch lock");
        hashes
            .iter()
            .map(|hash| {
                let state = map.entry(*hash).or_insert(WatchState::Watching);
                match state {
                    WatchState::Restored | WatchState::Rescued(_) => true,
                    _ => {
                        *state = WatchState::Deleting;
                        false
                    }
                }
            })
            .collect()
    }

    /// Finish deleting `hashes` after the delete batch committed. Returns the
    /// bytes of every node read or written during the delete.
    pub fn finish_delete(&self, hashes: &[Uint256]) -> Vec<(Uint256, Vec<u8>)> {
        let mut map = self.map.write().expect("restore watch lock");
        hashes
            .iter()
            .filter_map(|hash| match map.remove(hash) {
                Some(WatchState::Rescued(bytes)) => Some((*hash, bytes)),
                _ => None,
            })
            .collect()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.read().expect("restore watch lock").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u8) -> Uint256 {
        Uint256::from_array([n; 32])
    }

    #[test]
    fn read_of_watched_node_keeps_it() {
        let watch = RestoreWatch::new();
        watch.watch(&[h(1), h(2)]);
        let got = watch.read(&h(1), || ("bytes", Some(vec![1])));
        assert_eq!(got, "bytes");
        assert!(watch.was_restored(&h(1)));
        assert!(!watch.was_restored(&h(2)));
        assert_eq!(watch.begin_delete(&[h(1), h(2)]), vec![true, false]);
    }

    #[test]
    fn read_miss_does_not_pin() {
        let watch = RestoreWatch::new();
        watch.watch(&[h(1)]);
        watch.read(&h(1), || ((), None));
        assert!(!watch.was_restored(&h(1)));
    }

    #[test]
    fn read_during_delete_is_rescued() {
        let watch = RestoreWatch::new();
        watch.watch(&[h(1), h(2)]);
        assert_eq!(watch.begin_delete(&[h(1), h(2)]), vec![false, false]);
        // A reader got the bytes before the delete committed.
        watch.read(&h(1), || ((), Some(vec![7, 7])));
        let rescued = watch.finish_delete(&[h(1), h(2)]);
        assert_eq!(rescued, vec![(h(1), vec![7, 7])]);
        assert_eq!(watch.len(), 0);
    }

    #[test]
    fn write_during_delete_is_rescued_and_before_watch_is_kept() {
        let watch = RestoreWatch::new();
        watch.watch(&[h(1), h(2)]);
        watch.before_write(&h(2), || vec![2]);
        assert_eq!(watch.begin_delete(&[h(1), h(2)]), vec![false, true]);
        watch.before_write(&h(1), || vec![9]);
        assert_eq!(watch.finish_delete(&[h(1)]), vec![(h(1), vec![9])]);
    }

    #[test]
    fn unwatched_hashes_are_untouched() {
        let watch = RestoreWatch::new();
        watch.before_write(&h(5), || panic!("bytes must not be built"));
        assert_eq!(watch.read(&h(5), || (3, Some(vec![1]))), 3);
        assert_eq!(watch.len(), 0);
    }
}
