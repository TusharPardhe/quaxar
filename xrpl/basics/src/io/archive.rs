//! Parity surface for `xrpl/basics/Archive.h`.

use lz4_flex::frame::FrameDecoder;
use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;
use tar::Archive;

#[derive(Debug)]
pub enum ArchiveError {
    InvalidSourceFile,
    Io(io::Error),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceFile => write!(f, "Invalid source file"),
            Self::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<io::Error> for ArchiveError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn extract_tar_lz4(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<(), ArchiveError> {
    let src = src.as_ref();
    let dst = dst.as_ref();

    if !src.is_file() {
        return Err(ArchiveError::InvalidSourceFile);
    }

    std::fs::create_dir_all(dst)?;
    let file = File::open(src)?;
    let decoder = FrameDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive.unpack(dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ArchiveError, extract_tar_lz4};
    use lz4_flex::frame::FrameEncoder;
    use std::fs;
    use std::path::PathBuf;
    use tar::Builder;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[test]
    fn extract_tar_lz4_unpacks_archive_contents() {
        let root = unique_temp_dir("archive-test");
        let src_dir = root.join("src");
        let dst_dir = root.join("dst");
        fs::create_dir_all(&src_dir).expect("src dir");

        let file_path = src_dir.join("hello.txt");
        fs::write(&file_path, b"hello archive").expect("write test file");

        let archive_path = root.join("payload.tar.lz4");
        let file = fs::File::create(&archive_path).expect("create archive");
        let encoder = FrameEncoder::new(file);
        let mut builder = Builder::new(encoder);
        builder
            .append_path_with_name(&file_path, "hello.txt")
            .expect("append");
        let encoder = builder.into_inner().expect("finish tar");
        encoder.finish().expect("finish lz4");

        extract_tar_lz4(&archive_path, &dst_dir).expect("extract");
        assert_eq!(
            fs::read(dst_dir.join("hello.txt")).expect("read extracted"),
            b"hello archive"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn extract_tar_lz4_rejects_non_files() {
        let root = unique_temp_dir("archive-invalid");
        let error = extract_tar_lz4(&root, root.join("dst")).expect_err("invalid src");
        assert!(matches!(error, ArchiveError::InvalidSourceFile));
        let _ = fs::remove_dir_all(root);
    }
}
