use crate::paths::node::{Path, PathNode};
use crate::paths::path_rank::PathRanker;
use ledger::views::apply_view::ApplyView;
use protocol::{AccountID, Keylet, LedgerEntryType, STAmount, get_field_by_symbol};
use std::collections::{BTreeSet, VecDeque};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

pub struct Pathfinder {
    pub max_paths: usize,
    pub max_hops: usize,
}

impl Pathfinder {
    pub fn new(max_paths: usize, max_hops: usize) -> Self {
        Self {
            max_paths,
            max_hops,
        }
    }

    /// Core Pathfinder search loop (BFS-based graph traversal)
    /// Matches reference Pathfinder::findPaths
    pub fn find_paths(
        &self,
        view: &dyn ApplyView,
        src: AccountID,
        dst: AccountID,
        dst_amount: STAmount,
    ) -> Vec<Path> {
        let mut discovered_paths = Vec::new();
        let mut queue = VecDeque::new();
        let mut visited = BTreeSet::new();
        visited.insert(src);

        // Initial path from source
        let initial_path = Path {
            nodes: vec![PathNode::Account(src)],
            quality: STAmount::default(),
            liquidity: STAmount::default(),
        };
        queue.push_back(initial_path);

        while let Some(current_path) = queue.pop_front() {
            if current_path.nodes.len() > self.max_hops {
                continue;
            }

            let last_node = current_path.nodes.last().unwrap();

            // Check if we reached the destination
            if let PathNode::Account(acc) = last_node {
                if *acc == dst {
                    discovered_paths.push(current_path);
                    if discovered_paths.len() >= self.max_paths {
                        break;
                    }
                    continue;
                }
            }

            // Expand nodes
            self.expand_node(
                view,
                &current_path,
                &mut queue,
                &mut visited,
                &dst,
                &dst_amount,
            );
        }

        PathRanker::rank_paths(&mut discovered_paths);
        discovered_paths
    }

    fn expand_node(
        &self,
        view: &dyn ApplyView,
        path: &Path,
        queue: &mut VecDeque<Path>,
        visited: &mut BTreeSet<AccountID>,
        dst: &AccountID,
        dst_amount: &STAmount,
    ) {
        let last_node = path.nodes.last().unwrap();

        match last_node {
            PathNode::Account(acc) => {
                // 1. Expand via trust lines (Account -> Account)
                // Iterate owner directory to find RippleState entries
                let owner_dir =
                    protocol::owner_dir_keylet(basics::base_uint::Uint160::from_void(acc.data()));
                if let Ok(Some(dir_sle)) = view.read(owner_dir) {
                    let indexes = dir_sle.get_field_v256(sf("sfIndexes"));
                    for index in indexes.value() {
                        // Read each entry to check if it's a trust line
                        let entry_keylet = Keylet {
                            entry_type: LedgerEntryType::RippleState,
                            key: *index,
                        };
                        if let Ok(Some(entry)) = view.read(entry_keylet) {
                            if entry.get_type() == LedgerEntryType::RippleState {
                                // Extract the other account from the trust line
                                let low_limit = entry.get_field_amount(sf("sfLowLimit"));
                                let high_limit = entry.get_field_amount(sf("sfHighLimit"));
                                let low_account = low_limit.issue().account;
                                let high_account = high_limit.issue().account;
                                let other = if low_account == *acc {
                                    high_account
                                } else {
                                    low_account
                                };

                                // Don't revisit (except destination)
                                if other != *dst && visited.contains(&other) {
                                    continue;
                                }

                                // Add path through this account
                                let mut new_path = path.clone();
                                new_path.nodes.push(PathNode::Account(other));
                                queue.push_back(new_path);
                                visited.insert(other);
                            }
                        }
                    }
                }

                // 2. Expand via order books (DestBook/Books node types)
                // Check what currencies the source account holds, then look for
                // books from those currencies to the destination currency.
                if !dst_amount.native() {
                    let dst_issue = dst_amount.issue();

                    // Scan the account's trust lines to find currencies it holds
                    let owner_dir2 = protocol::owner_dir_keylet(
                        basics::base_uint::Uint160::from_void(acc.data()),
                    );
                    if let Ok(Some(dir_sle)) = view.read(owner_dir2) {
                        let indexes = dir_sle.get_field_v256(sf("sfIndexes"));
                        for index in indexes.value() {
                            let entry_keylet = Keylet {
                                entry_type: LedgerEntryType::RippleState,
                                key: *index,
                            };
                            if let Ok(Some(entry)) = view.read(entry_keylet) {
                                if entry.get_type() == LedgerEntryType::RippleState {
                                    let low_limit = entry.get_field_amount(sf("sfLowLimit"));
                                    let high_limit = entry.get_field_amount(sf("sfHighLimit"));
                                    let balance = entry.get_field_amount(sf("sfBalance"));
                                    let low_account = low_limit.issue().account;
                                    let currency = low_limit.issue().currency;
                                    let issuer = if low_account == *acc {
                                        high_limit.issue().account
                                    } else {
                                        low_account
                                    };

                                    // Check source holds this currency (positive balance)
                                    let src_is_low = low_account == *acc;
                                    let has_balance = if src_is_low {
                                        balance.signum() > 0
                                    } else {
                                        balance.signum() < 0
                                    };

                                    if !has_balance {
                                        continue;
                                    }

                                    // Skip if same currency as destination
                                    if currency == dst_issue.currency && issuer == dst_issue.account
                                    {
                                        continue;
                                    }

                                    // DestBook: check if a book exists from
                                    // this currency to the destination currency.
                                    // Use succ() on the book base key (same as
                                    // BookTip step iterates the book
                                    // directory using the quality-keyed SHAMap).
                                    let src_asset = protocol::Asset::Issue(protocol::Issue::new(
                                        currency, issuer,
                                    ));
                                    let dst_asset = protocol::Asset::Issue(dst_issue);
                                    let book = protocol::Book::new(src_asset, dst_asset, None);
                                    let book_base = protocol::keylet::book(book).key;
                                    let book_end = {
                                        let mut end = book_base;
                                        let bytes = end.data_mut();
                                        for b in bytes[24..32].iter_mut() {
                                            *b = 0xFF;
                                        }
                                        end
                                    };
                                    if let Ok(Some(_)) = view.succ(book_base, Some(book_end)) {
                                        // Book exists! Add path: source → book(dst_issue) → dst
                                        if !visited.contains(dst) {
                                            let mut new_path = path.clone();
                                            new_path.nodes.push(PathNode::OrderBook(dst_issue));
                                            new_path.nodes.push(PathNode::Account(*dst));
                                            queue.push_back(new_path);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Also check XRP bridge: source XRP → book(XRP→dst) → dst
                    if !visited.contains(dst) {
                        let xrp_asset = protocol::Asset::Issue(protocol::xrp_issue());
                        let dst_asset = protocol::Asset::Issue(dst_issue);
                        let book = protocol::Book::new(xrp_asset, dst_asset, None);
                        let book_base = protocol::keylet::book(book).key;
                        let book_end = {
                            let mut end = book_base;
                            let bytes = end.data_mut();
                            for b in bytes[24..32].iter_mut() {
                                *b = 0xFF;
                            }
                            end
                        };
                        if let Ok(Some(_)) = view.succ(book_base, Some(book_end)) {
                            let mut new_path = path.clone();
                            new_path.nodes.push(PathNode::OrderBook(dst_issue));
                            new_path.nodes.push(PathNode::Account(*dst));
                            queue.push_back(new_path);
                        }
                    }
                }
            }
            PathNode::OrderBook(issue) => {
                // From a book node, we can reach the issuer of the output currency
                let issuer = issue.account;
                if !visited.contains(&issuer) {
                    let mut new_path = path.clone();
                    new_path.nodes.push(PathNode::Account(issuer));
                    queue.push_back(new_path);
                    visited.insert(issuer);
                }
            }
        }
    }
}
