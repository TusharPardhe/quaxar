//! Rust port of `xrpl::BookDirs` from `xrpl/ledger/BookDirs.h`.

use std::sync::Arc;

use basics::base_uint::Uint256;
use protocol::{
    Book, STLedgerEntry, directory_node_keylet, get_book_base, get_quality_next,
    offer_keylet_from_key,
};

use crate::directory::{Dir, DirIter};
use crate::read_view::ReadView;

pub struct BookDirs<'a> {
    view: &'a dyn ReadView,
    root: Uint256,
    next_quality: Uint256,
    key: Uint256,
}

impl<'a> BookDirs<'a> {
    pub fn new(view: &'a dyn ReadView, book: &Book) -> Self {
        let root = get_book_base(*book);
        let next_quality = get_quality_next(root);
        let key = view
            .succ(root, Some(next_quality))
            .unwrap_or(None)
            .unwrap_or(Uint256::zero());

        Self {
            view,
            root,
            next_quality,
            key,
        }
    }

    pub fn iter(&self) -> BookDirIter<'a> {
        let mut it = BookDirIter {
            view: self.view,
            root: self.root,
            next_quality: self.next_quality,
            cur_key: self.key,
            dir_iter: None,
        };

        if self.key.is_non_zero()
            && let Ok(dir) = Dir::new(self.view, directory_node_keylet(self.key))
        {
            it.dir_iter = Some(dir.begin());
        }

        it
    }
}

pub struct BookDirIter<'a> {
    view: &'a dyn ReadView,
    #[allow(dead_code)]
    root: Uint256,
    next_quality: Uint256,
    cur_key: Uint256,
    dir_iter: Option<DirIter<'a>>,
}

impl<'a> Iterator for BookDirIter<'a> {
    type Item = Arc<STLedgerEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut it) = self.dir_iter
                && !it.is_end()
            {
                let index = it.index();
                let offer = self.view.read(offer_keylet_from_key(index)).unwrap_or(None);
                if it.advance().is_err() {
                    self.dir_iter = None;
                }
                if let Some(offer) = offer {
                    return Some(offer);
                }
                continue;
            }

            if self.cur_key.is_zero() {
                return None;
            }

            self.cur_key = self
                .view
                .succ(self.cur_key, Some(self.next_quality))
                .unwrap_or(None)
                .unwrap_or(Uint256::zero());
            if self.cur_key.is_zero() {
                return None;
            }

            if let Ok(dir) = Dir::new(self.view, directory_node_keylet(self.cur_key)) {
                self.dir_iter = Some(dir.begin());
            } else {
                return None;
            }
        }
    }
}
