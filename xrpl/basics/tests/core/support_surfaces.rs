use basics::archive::extract_tar_lz4;
use basics::base_uint::Uint256;
use basics::compression_algorithms::{lz4_compress, lz4_decompress};
use basics::decaying_sample::{DecayWindow, DecayingSample};
use basics::key_cache::{KeyCache, ManualClock};
use basics::log::{LogSeverity, Logs, RecordingLogSink};
use basics::make_ssl_context::{
    DEFAULT_CIPHER_LIST, SslContextMode, SslVerifyMode, make_SSLContext, make_SSLContextAuthed,
};
use basics::malloc_trim::{NullMallocTrimLogger, malloc_trim};
use basics::mutex::Mutex;
use basics::resolver::Resolver;
use basics::resolver_asio::ResolverAsio;
use basics::rocksdb::rocksdb_available;
use basics::sanitizers::NO_SANITIZE_ADDRESS_SUPPORTED;
use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::rsa::Rsa;
use openssl::x509::{X509, X509NameBuilder};
use std::fs;
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::time::{Duration, Instant};
use time::Duration as TimeDuration;

fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&path).expect("temp dir");
    path
}

#[test]
fn basics_support_decaying_sample_matches_current_cpp_window_shape() {
    let start = Instant::now();
    let mut sample = DecayingSample::<32>::new(start);
    assert_eq!(sample.add(3200, start), 100);
    assert_eq!(sample.value(start + Duration::from_secs(1)), 96);

    let mut window = DecayWindow::<10>::new(start);
    window.add(20.0, start);
    assert!((window.value(start + Duration::from_secs(10)) - 1.0).abs() < 1e-9);
}

#[test]
fn basics_support_key_cache_alias_is_uint256_key_only_cache() {
    let clock = ManualClock::new(0);
    let cache = KeyCache::new("keys", 4, TimeDuration::seconds(60), clock);
    let key = Uint256::from_hex("0102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F20")
        .expect("hex");

    assert!(cache.insert(key));
    assert!(!cache.insert(key));
    assert_eq!(cache.size(), 1);
}

#[test]
fn basics_support_lz4_and_archive_helpers_match_cpp_shape() {
    let input = b"support surface payload";
    let (mut compressed, size) =
        lz4_compress(input, |capacity| vec![0_u8; capacity]).expect("compress");
    compressed.truncate(size);
    let mut decompressed = vec![0_u8; input.len()];
    assert_eq!(
        lz4_decompress(&compressed, &mut decompressed).expect("decompress"),
        input.len()
    );
    assert_eq!(decompressed, input);

    let root = unique_temp_dir("archive-support");
    let source = root.join("source");
    let target = root.join("target");
    fs::create_dir_all(&source).expect("source dir");
    let nested = source.join("payload.txt");
    fs::write(&nested, input).expect("write payload");

    let archive_path = root.join("payload.tar.lz4");
    {
        let file = fs::File::create(&archive_path).expect("archive file");
        let encoder = lz4_flex::frame::FrameEncoder::new(file);
        let mut builder = tar::Builder::new(encoder);
        builder
            .append_path_with_name(&nested, "payload.txt")
            .expect("append");
        let encoder = builder.into_inner().expect("finish tar");
        encoder.finish().expect("finish lz4");
    }

    extract_tar_lz4(&archive_path, &target).expect("extract");
    assert_eq!(
        fs::read(target.join("payload.txt")).expect("read extracted"),
        input
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn basics_support_log_malloc_and_platform_flags_are_observable() {
    let sink = Arc::new(RecordingLogSink::default());
    let logs = Logs::with_sink(LogSeverity::Info, sink.clone());
    logs.write(LogSeverity::Info, "unit", "visible", false);
    assert_eq!(sink.entries().len(), 1);

    let report = malloc_trim("support", &NullMallocTrimLogger);
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    assert!(report.supported);
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    assert!(!report.supported);

    let _ = NO_SANITIZE_ADDRESS_SUPPORTED;
    assert!(rocksdb_available());
}

#[test]
fn basics_support_ssl_and_resolver_match_current_cpp_runtime_shape() {
    let anonymous = make_SSLContext("").expect("anonymous ssl");
    assert_eq!(anonymous.mode(), SslContextMode::Anonymous);
    assert_eq!(anonymous.verify_mode(), SslVerifyMode::None);
    assert_eq!(anonymous.cipher_list(), DEFAULT_CIPHER_LIST);

    let root = unique_temp_dir("ssl-resolver-support");
    let cert = root.join("server.cert");
    let key = root.join("server.key");
    let identity_key = PKey::from_rsa(Rsa::generate(2048).expect("rsa")).expect("pkey");
    let mut identity = X509::builder().expect("x509");
    identity.set_version(2).expect("version");
    let mut name = X509NameBuilder::new().expect("name builder");
    name.append_entry_by_text("CN", "localhost").expect("cn");
    let name = name.build();
    identity.set_subject_name(&name).expect("subject");
    identity.set_issuer_name(&name).expect("issuer");
    identity.set_pubkey(&identity_key).expect("pubkey");
    identity
        .set_not_before(&Asn1Time::days_from_now(0).expect("not before"))
        .expect("set not before");
    identity
        .set_not_after(&Asn1Time::days_from_now(30).expect("not after"))
        .expect("set not after");
    identity
        .sign(&identity_key, MessageDigest::sha256())
        .expect("sign");
    let identity = identity.build();
    fs::write(&cert, identity.to_pem().expect("cert pem")).expect("cert");
    fs::write(
        &key,
        identity_key.private_key_to_pem_pkcs8().expect("key pem"),
    )
    .expect("key");
    let authed = make_SSLContextAuthed(&key, &cert, std::path::PathBuf::new(), "HIGH:!aNULL")
        .expect("authed");
    assert_eq!(authed.mode(), SslContextMode::Authenticated);
    assert_eq!(authed.verify_mode(), SslVerifyMode::None);

    let resolver = ResolverAsio::default();
    resolver.start();
    let seen = Arc::new((StdMutex::new(Vec::new()), Condvar::new()));
    let seen_handler = Arc::clone(&seen);
    resolver.resolve(
        &["127.0.0.1:6000".to_owned()],
        Arc::new(move |name, endpoints| {
            let (lock, cv) = &*seen_handler;
            lock.lock()
                .expect("seen mutex poisoned")
                .push((name, endpoints));
            cv.notify_all();
        }),
    );
    let (lock, cv) = &*seen;
    let seen = cv
        .wait_timeout_while(
            lock.lock().expect("seen mutex poisoned"),
            Duration::from_secs(2),
            |entries| entries.is_empty(),
        )
        .expect("wait for resolver")
        .0;
    assert_eq!(seen[0].0, "127.0.0.1:6000");
    assert_eq!(seen[0].1[0].port(), 6000);
    resolver.stop();

    let _ = fs::remove_dir_all(root);
}

#[test]
fn basics_mutex_supports_cpp_style_construction_and_lock_access() {
    let default_mutex = Mutex::<i32>::make();
    assert_eq!(*default_mutex.lock(), 0);

    let number_mutex = Mutex::<i32>::make_from(42);
    assert_eq!(*number_mutex.lock(), 42);

    let string_mutex = Mutex::<String>::make_from("test".to_owned());
    assert_eq!(string_mutex.lock().get(), "test");

    let move_only_mutex = Mutex::<Box<i32>>::make_from(Box::new(100));
    assert_eq!(**move_only_mutex.lock(), 100);

    #[derive(Debug, PartialEq, Eq)]
    struct Data {
        x: i32,
        y: String,
    }

    let custom = Mutex::<Data>::make_with(|| Data {
        x: 7,
        y: "hello".to_owned(),
    });
    let lock = custom.lock();
    assert_eq!(lock.get().x, 7);
    assert_eq!(lock.get().y, "hello");
}

#[test]
fn basics_mutex_supports_rwlock_shared_and_exclusive_access() {
    let mutex = Mutex::<i32, std::sync::RwLock<i32>>::new(100);

    {
        let lock = mutex.lock_shared();
        assert_eq!(*lock, 100);
    }

    {
        let mut lock = mutex.lock();
        *lock = 200;
    }

    assert_eq!(*mutex.lock_shared(), 200);
}
