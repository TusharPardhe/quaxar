#[cfg(test)]
mod tests {
    use crate::crypto::seed::parse_base58_seed;
    #[test]
    fn test_sed_parsing() {
        let s = "sEd7ZFrv3FDZM6W8zcBL7qDuBTVwGHP";
        let seed = parse_base58_seed(s);
        assert!(seed.is_some(), "Should parse sEd seed");
    }
}
