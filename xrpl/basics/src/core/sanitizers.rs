//! Compatibility helpers for `xrpl/basics/sanitizers.h`.

pub const NO_SANITIZE_ADDRESS_SUPPORTED: bool = cfg!(any(target_env = "gnu", target_env = "musl"));

#[macro_export]
macro_rules! xrpl_no_sanitize_address {
    ($item:item) => {
        $item
    };
}

#[cfg(test)]
mod tests {
    use super::NO_SANITIZE_ADDRESS_SUPPORTED;

    #[test]
    fn no_sanitize_flag_is_runtime_observable() {
        let expected = cfg!(any(target_env = "gnu", target_env = "musl"));
        assert_eq!(NO_SANITIZE_ADDRESS_SUPPORTED, expected);
    }
}
