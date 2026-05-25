//! Rust equivalent of `xrpl/basics/Blob.h`.

/// Storage for linear binary data.
pub type Blob = Vec<u8>;

#[cfg(test)]
mod tests {
    use super::Blob;

    #[test]
    fn blob_is_vector_backed_binary_storage() {
        let mut blob: Blob = vec![0xde, 0xad];
        blob.push(0xbe);
        blob.push(0xef);

        assert_eq!(blob, vec![0xde, 0xad, 0xbe, 0xef]);
    }
}
