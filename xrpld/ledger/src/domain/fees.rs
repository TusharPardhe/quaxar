//! `xrpl/protocol/Fees.h` compatibility surface.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fees {
    pub base: u64,
    pub reserve: u64,
    pub increment: u64,
}

impl Fees {
    pub fn account_reserve(&self, owner_count: usize) -> u64 {
        self.account_reserve_with_account_count(owner_count, 1)
    }

    pub fn account_reserve_with_account_count(
        &self,
        owner_count: usize,
        account_count: usize,
    ) -> u64 {
        let owner_count =
            u64::try_from(owner_count).expect("owner count must fit within the fee range");
        let account_count =
            u64::try_from(account_count).expect("account count must fit within the fee range");
        account_count * self.reserve + owner_count * self.increment
    }
}

#[cfg(test)]
mod tests {
    use super::Fees;

    #[test]
    fn account_reserve_matches_current_cpp_formula() {
        let fees = Fees {
            base: 10,
            reserve: 200,
            increment: 50,
        };

        assert_eq!(fees.account_reserve(0), 200);
        assert_eq!(fees.account_reserve(1), 250);
        assert_eq!(fees.account_reserve(3), 350);
        assert_eq!(fees.account_reserve_with_account_count(3, 0), 150);
        assert_eq!(fees.account_reserve_with_account_count(3, 2), 550);
    }
}
