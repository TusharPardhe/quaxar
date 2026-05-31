//! `Book` helpers ported from `xrpl/protocol/Book.*`.

use crate::{Asset, Domain, is_consistent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Book {
    pub r#in: Asset,
    pub out: Asset,
    pub domain: Option<Domain>,
}

impl Book {
    pub fn new(r#in: impl Into<Asset>, out: impl Into<Asset>, domain: Option<Domain>) -> Self {
        Self {
            r#in: r#in.into(),
            out: out.into(),
            domain,
        }
    }

    pub fn text(&self) -> String {
        format!("{}->{}", self.r#in.text(), self.out.text())
    }
}

pub fn is_consistent_book(book: Book) -> bool {
    let in_consistent = match book.r#in {
        Asset::Issue(issue) => is_consistent(issue),
        Asset::MPTIssue(issue) => !issue.issuer().is_zero(),
    };
    let out_consistent = match book.out {
        Asset::Issue(issue) => is_consistent(issue),
        Asset::MPTIssue(issue) => !issue.issuer().is_zero(),
    };
    in_consistent && out_consistent && book.r#in != book.out
}

pub fn reverse_book(book: Book) -> Book {
    Book::new(book.out, book.r#in, book.domain)
}

#[cfg(test)]
mod tests {
    use crate::{AccountID, Currency, Domain, Issue};

    use super::{Book, is_consistent_book, reverse_book};

    #[test]
    fn consistency_book_rules() {
        let left = Issue::new(
            Currency::from_hex("0102030405060708090A0B0C0D0E0F1011121314").expect("currency"),
            AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").expect("account"),
        );
        let right = Issue::new(
            Currency::from_hex("1111111111111111111111111111111111111111").expect("currency"),
            AccountID::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB").expect("account"),
        );

        assert!(is_consistent_book(Book::new(left, right, None)));
        assert!(!is_consistent_book(Book::new(left, left, None)));
    }

    #[test]
    fn reverse_preserves_domain_and_swaps_issues() {
        let left = Issue::new(
            Currency::from_hex("0102030405060708090A0B0C0D0E0F1011121314").expect("currency"),
            AccountID::from_hex("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").expect("account"),
        );
        let right = Issue::new(
            Currency::from_hex("1111111111111111111111111111111111111111").expect("currency"),
            AccountID::from_hex("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB").expect("account"),
        );
        let domain =
            Domain::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
                .expect("domain");
        let book = Book::new(left, right, Some(domain));

        assert_eq!(reverse_book(book), Book::new(right, left, Some(domain)));
    }
}
