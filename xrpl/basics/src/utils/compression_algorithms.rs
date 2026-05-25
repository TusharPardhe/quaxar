//! Parity surface for `xrpl/basics/CompressionAlgorithms.h`.

use std::fmt;
use std::io::{self, Read};

#[derive(Debug)]
pub enum CompressionError {
    InvalidSize,
    BufferTooSmall,
    CompressFailed,
    DecompressFailed,
    InsufficientInputSize,
    Io(io::Error),
}

impl fmt::Display for CompressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize => write!(f, "lz4 compress: invalid size"),
            Self::BufferTooSmall => write!(f, "lz4 compress: output buffer too small"),
            Self::CompressFailed => write!(f, "lz4 compress: failed"),
            Self::DecompressFailed => write!(f, "lz4Decompress: failed"),
            Self::InsufficientInputSize => {
                write!(f, "lz4 decompress: insufficient input size")
            }
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CompressionError {}

impl From<io::Error> for CompressionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn lz4_compress<F, B>(input: &[u8], buffer_factory: F) -> Result<(B, usize), CompressionError>
where
    F: FnOnce(usize) -> B,
    B: AsMut<[u8]>,
{
    if input.len() > u32::MAX as usize {
        return Err(CompressionError::InvalidSize);
    }

    let out_capacity = lz4_flex::block::get_maximum_output_size(input.len());
    let mut compressed = buffer_factory(out_capacity);
    if compressed.as_mut().len() < out_capacity {
        return Err(CompressionError::BufferTooSmall);
    }

    let compressed_size = lz4_flex::block::compress_into(input, compressed.as_mut())
        .map_err(|_| CompressionError::CompressFailed)?;
    if compressed_size == 0 {
        return Err(CompressionError::CompressFailed);
    }

    Ok((compressed, compressed_size))
}

pub fn lz4_decompress(input: &[u8], decompressed: &mut [u8]) -> Result<usize, CompressionError> {
    if input.is_empty() || decompressed.is_empty() {
        return Err(CompressionError::DecompressFailed);
    }

    let size = lz4_flex::block::decompress_into(input, decompressed)
        .map_err(|_| CompressionError::DecompressFailed)?;
    if size != decompressed.len() {
        return Err(CompressionError::DecompressFailed);
    }
    Ok(size)
}

pub fn lz4_decompress_reader<R: Read>(
    reader: &mut R,
    input_size: usize,
    decompressed: &mut [u8],
) -> Result<usize, CompressionError> {
    let mut compressed = vec![0_u8; input_size];
    reader
        .read_exact(&mut compressed)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => CompressionError::InsufficientInputSize,
            _ => CompressionError::Io(error),
        })?;

    lz4_decompress(&compressed, decompressed)
}

#[cfg(test)]
mod tests {
    use super::{CompressionError, lz4_compress, lz4_decompress, lz4_decompress_reader};
    use std::io::Cursor;

    #[test]
    fn lz4_round_trip_block_shape() {
        let input = b"the quick brown fox jumps over the lazy dog";
        let (mut compressed, size) =
            lz4_compress(input, |capacity| vec![0_u8; capacity]).expect("compress");
        compressed.truncate(size);

        let mut decompressed = vec![0_u8; input.len()];
        assert_eq!(
            lz4_decompress(compressed.as_ref(), &mut decompressed).expect("decompress"),
            input.len()
        );
        assert_eq!(decompressed, input);
    }

    #[test]
    fn lz4_reader_decompress_requires_exact_input_size() {
        let input = b"reader block payload";
        let (mut compressed, size) =
            lz4_compress(input, |capacity| vec![0_u8; capacity]).expect("compress");
        compressed.truncate(size);

        let mut cursor = Cursor::new(compressed.clone());
        let mut decompressed = vec![0_u8; input.len()];
        assert_eq!(
            lz4_decompress_reader(&mut cursor, size, &mut decompressed).expect("reader decompress"),
            input.len()
        );
        assert_eq!(decompressed, input);

        let mut short = Cursor::new(compressed[..size - 1].to_vec());
        let error = lz4_decompress_reader(&mut short, size, &mut decompressed).expect_err("short");
        assert!(matches!(error, CompressionError::InsufficientInputSize));
    }
}
