use basics::basic_config::Section;
use nodestore::{
    JournalLevel, NUDB_APPNUM, NUDB_CURRENT_VERSION, NUDB_DATA_FILE_TYPE, NUDB_DEFAULT_BLOCK_SIZE,
    NUDB_KEY_FILE_TYPE, NUDB_LOG_FILE_TYPE, NodeStoreJournal, NuDbBackendConfig,
    NuDbDataFileHeader, NuDbFileSetState, NuDbKeyFileHeader, NuDbLayout, NuDbLogFileHeader,
    NuDbOpenAction, NuDbOpenArgs, encode_nudb_data_file_header, encode_nudb_key_file_header,
    encode_nudb_log_file_header, nudb_bucket_capacity, nudb_encode_load_factor, nudb_pepper,
    parse_nudb_block_size, read_nudb_data_file_header, read_nudb_key_file_header,
    read_nudb_log_file_header,
};
use std::sync::Mutex;
use tempfile::TempDir;

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<(JournalLevel, String)>>,
}

impl RecordingJournal {
    fn take(&self) -> Vec<(JournalLevel, String)> {
        self.entries
            .lock()
            .expect("recording journal mutex must not be poisoned")
            .clone()
    }
}

impl NodeStoreJournal for RecordingJournal {
    fn log(&self, level: JournalLevel, message: &str) {
        self.entries
            .lock()
            .expect("recording journal mutex must not be poisoned")
            .push((level, message.to_owned()));
    }
}

#[test]
fn nudb_foundation_uses_cpp_file_names_and_default_appnum() {
    let layout = NuDbLayout::from_base_path("/tmp/nudb-foundation");
    assert!(layout.data_path.ends_with("nudb.dat"));
    assert!(layout.key_path.ends_with("nudb.key"));
    assert!(layout.log_path.ends_with("nudb.log"));

    let open_args = NuDbOpenArgs::xrpld_default(123, 456);
    assert_eq!(open_args.app_type, NUDB_APPNUM);
    assert_eq!(open_args.uid, 123);
    assert_eq!(open_args.salt, 456);
}

#[test]
fn nudb_foundation_preserves_cpp_block_size_parsing_and_logging() {
    let mut section = Section::new("node_db");
    section.set("path", "/tmp/nudb-foundation");
    section.set("nudb_block_size", "8192");
    let journal = RecordingJournal::default();

    let config =
        NuDbBackendConfig::from_section(nodestore::NodeObject::KEY_BYTES, &section, 64, &journal)
            .expect("config");

    assert_eq!(config.block_size, 8192);
    assert_eq!(
        journal.take(),
        vec![(
            JournalLevel::Info,
            "Using custom NuDB block size: 8192 bytes".to_owned()
        )]
    );
}

#[test]
fn nudb_foundation_defaults_to_4k_without_override() {
    let mut section = Section::new("node_db");
    section.set("path", "/tmp/nudb-foundation");
    let journal = RecordingJournal::default();

    let parsed = parse_nudb_block_size(&section, &journal).expect("default block size");
    assert_eq!(parsed, NUDB_DEFAULT_BLOCK_SIZE);
    assert!(journal.take().is_empty());
}

#[test]
fn nudb_foundation_open_plan_rejects_partial_file_sets_and_preserves_create_open_split() {
    let temp = TempDir::new().expect("tempdir");
    let mut section = Section::new("node_db");
    section.set("path", temp.path().to_string_lossy().into_owned());
    let journal = RecordingJournal::default();
    let config =
        NuDbBackendConfig::from_section(nodestore::NodeObject::KEY_BYTES, &section, 64, &journal)
            .expect("config");

    let create_plan = config
        .build_open_plan(true, NuDbOpenArgs::deterministic(7, 8, 9))
        .expect("create plan");
    assert_eq!(create_plan.action, NuDbOpenAction::CreateNew);

    std::fs::create_dir_all(temp.path()).expect("dir");
    std::fs::write(temp.path().join("nudb.dat"), []).expect("partial data");
    assert_eq!(config.layout.file_set_state(), NuDbFileSetState::Partial);
    assert!(
        config
            .build_open_plan(true, NuDbOpenArgs::deterministic(7, 8, 9))
            .expect_err("partial should fail")
            .contains("Incomplete NuDB file set")
    );
}

#[test]
fn nudb_foundation_reads_exact_cpp_shaped_key_header_bytes() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("nudb.key");
    let header = NuDbKeyFileHeader {
        version: NUDB_CURRENT_VERSION,
        uid: 10,
        appnum: NUDB_APPNUM,
        key_size: 32,
        salt: 20,
        pepper: nudb_pepper(20),
        block_size: 4096,
        load_factor: nudb_encode_load_factor(0.5).expect("load factor"),
        capacity: nudb_bucket_capacity(4096),
        buckets: 0,
        modulus: 1,
    };
    let bytes = encode_nudb_key_file_header(&header).expect("encode");
    assert_eq!(&bytes[..8], NUDB_KEY_FILE_TYPE);
    std::fs::write(&path, bytes).expect("write");

    let read = read_nudb_key_file_header(&path).expect("read");
    assert_eq!(read.uid, 10);
    assert_eq!(read.appnum, NUDB_APPNUM);
    assert_eq!(read.salt, 20);
    assert_eq!(read.pepper, nudb_pepper(20));
    assert_eq!(read.block_size, 4096);
}

#[test]
fn nudb_foundation_reads_exact_cpp_shaped_data_and_log_header_bytes() {
    let temp = TempDir::new().expect("tempdir");
    let data_path = temp.path().join("nudb.dat");
    let log_path = temp.path().join("nudb.log");
    let data_header = NuDbDataFileHeader {
        version: NUDB_CURRENT_VERSION,
        uid: 10,
        appnum: NUDB_APPNUM,
        key_size: 32,
    };
    let log_header = NuDbLogFileHeader {
        version: NUDB_CURRENT_VERSION,
        uid: 10,
        appnum: NUDB_APPNUM,
        key_size: 32,
        salt: 20,
        pepper: nudb_pepper(20),
        block_size: 4096,
        key_file_size: 8192,
        dat_file_size: 92,
    };
    let data_bytes = encode_nudb_data_file_header(&data_header).expect("data encode");
    let log_bytes = encode_nudb_log_file_header(&log_header).expect("log encode");
    assert_eq!(&data_bytes[..8], NUDB_DATA_FILE_TYPE);
    assert_eq!(&log_bytes[..8], NUDB_LOG_FILE_TYPE);
    std::fs::write(&data_path, data_bytes).expect("write data");
    std::fs::write(&log_path, log_bytes).expect("write log");

    let read_data = read_nudb_data_file_header(&data_path).expect("read data");
    let read_log = read_nudb_log_file_header(&log_path).expect("read log");
    assert_eq!(read_data, data_header);
    assert_eq!(read_log, log_header);
}
