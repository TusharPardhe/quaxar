use std::env;
use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    unsafe {
        env::set_var("PROTOC", protoc);
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let proto_root = workspace_root.join("xrpl/protocol/proto").canonicalize()?;
    let proto_file = proto_root.join("xrpl.proto");

    println!("cargo:rerun-if-changed={}", proto_file.display());

    prost_build::Config::new().compile_protos(&[proto_file], &[proto_root])?;
    Ok(())
}
