use crate::{
    Backend, BatchWriter, DecodedBlob, EncodedBlob, Factory, JournalLevel, NodeObject,
    NodeStoreJournal, Scheduler, Status,
};
use basics::rocksdb::{
    BlockBasedIndexType, BlockBasedOptions, Cache, ChecksumType, DBCompactionStyle,
    DBCompressionType, DBWithThreadMode, DataBlockIndexType, Env, ErrorKind, IteratorMode,
    MultiThreaded, Options, WriteBatch,
};
use basics::{base_uint::Uint256, basic_config::Section, rocksdb};
use std::{
    fmt::Display,
    fs,
    path::Path,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

fn read_string(section: &Section, key: &str) -> String {
    section
        .get::<String>(key)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn read_optional_string(section: &Section, key: &str) -> Result<Option<String>, String> {
    let value = section
        .get::<String>(key)
        .map_err(|error| format!("Invalid {key} value in RocksDBFactory backend: {error}"))?;

    Ok(value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    }))
}

fn read_optional_parsed<T>(section: &Section, key: &str) -> Result<Option<T>, String>
where
    T: FromStr,
    T::Err: Display,
{
    let Some(raw_value) = section
        .get::<String>(key)
        .map_err(|error| format!("Invalid {key} value in RocksDBFactory backend: {error}"))?
    else {
        return Ok(None);
    };

    raw_value.parse::<T>().map(Some).map_err(|error| {
        format!("Invalid {key} value in RocksDBFactory backend: {raw_value}. Error: {error}")
    })
}

fn read_optional_bool_like(section: &Section, key: &str) -> Result<Option<bool>, String> {
    let Some(raw_value) = section
        .get::<String>(key)
        .map_err(|error| format!("Invalid {key} value in RocksDBFactory backend: {error}"))?
    else {
        return Ok(None);
    };

    if let Ok(parsed) = raw_value.parse::<bool>() {
        return Ok(Some(parsed));
    }

    raw_value
        .parse::<i32>()
        .map(|parsed| Some(parsed != 0))
        .map_err(|error| {
            format!("Invalid {key} value in RocksDBFactory backend: {raw_value}. Error: {error}")
        })
}

fn megabytes(value: usize) -> usize {
    value * 1024 * 1024
}

fn parse_bool_like(value: &str) -> Result<bool, String> {
    if let Ok(parsed) = value.parse::<bool>() {
        return Ok(parsed);
    }

    value
        .parse::<i32>()
        .map(|parsed| parsed != 0)
        .map_err(|error| {
            format!("Invalid boolean value in RocksDBFactory backend: {value}. Error: {error}")
        })
}

fn parse_size_like(value: &str) -> Result<usize, String> {
    let trimmed = value.trim();
    let upper = trimmed.to_ascii_uppercase();

    let (number, multiplier) = if let Some(number) = upper.strip_suffix("KIB") {
        (number, 1024usize)
    } else if let Some(number) = upper.strip_suffix("KB") {
        (number, 1024usize)
    } else if let Some(number) = upper.strip_suffix('K') {
        (number, 1024usize)
    } else if let Some(number) = upper.strip_suffix("MIB") {
        (number, 1024usize * 1024)
    } else if let Some(number) = upper.strip_suffix("MB") {
        (number, 1024usize * 1024)
    } else if let Some(number) = upper.strip_suffix('M') {
        (number, 1024usize * 1024)
    } else if let Some(number) = upper.strip_suffix("GIB") {
        (number, 1024usize * 1024 * 1024)
    } else if let Some(number) = upper.strip_suffix("GB") {
        (number, 1024usize * 1024 * 1024)
    } else if let Some(number) = upper.strip_suffix('G') {
        (number, 1024usize * 1024 * 1024)
    } else {
        (trimmed, 1usize)
    };

    let parsed = number.trim().parse::<usize>().map_err(|error| {
        format!("Invalid size value in RocksDBFactory backend: {value}. Error: {error}")
    })?;

    parsed
        .checked_mul(multiplier)
        .ok_or_else(|| format!("Size value overflow in RocksDBFactory backend: {value}"))
}

fn parse_db_option_pairs(raw_value: &str) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::new();
    let mut segment = String::new();
    let mut brace_depth = 0usize;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let trimmed = raw_value.trim();
    let trimmed = trimmed
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(trimmed);

    for ch in trimmed.chars() {
        match ch {
            '{' if !in_single_quote && !in_double_quote => {
                brace_depth += 1;
                segment.push(ch);
            }
            '}' if !in_single_quote && !in_double_quote => {
                brace_depth = brace_depth.saturating_sub(1);
                segment.push(ch);
            }
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
                segment.push(ch);
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                segment.push(ch);
            }
            ';' if brace_depth == 0 && !in_single_quote && !in_double_quote => {
                if let Some(pair) = parse_db_option_segment(&segment)? {
                    pairs.push(pair);
                }
                segment.clear();
            }
            _ => segment.push(ch),
        }
    }

    if let Some(pair) = parse_db_option_segment(&segment)? {
        pairs.push(pair);
    }

    Ok(pairs)
}

fn parse_db_option_segment(segment: &str) -> Result<Option<(String, String)>, String> {
    let trimmed = segment.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let Some((key, value)) = trimmed.split_once('=') else {
        return Err(format!(
            "Invalid RocksDB option segment in RocksDBFactory backend: {trimmed}"
        ));
    };

    let key = key.trim();
    if key.is_empty() {
        return Err(format!(
            "Invalid RocksDB option segment in RocksDBFactory backend: {trimmed}"
        ));
    }

    Ok(Some((
        key.to_owned(),
        strip_option_quotes(value.trim()).to_owned(),
    )))
}

fn strip_option_quotes(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some(value) = trimmed
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
    {
        return value.trim();
    }

    if let Some(value) = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return value.trim();
    }

    if let Some(value) = trimmed
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return value.trim();
    }

    trimmed
}

fn parse_data_block_index_type(value: &str) -> Result<DataBlockIndexType, String> {
    let normalized = value
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "");
    match normalized.as_str() {
        "binarysearch" | "binary" => Ok(DataBlockIndexType::BinarySearch),
        "binaryandhash" | "binaryhash" | "hash" => Ok(DataBlockIndexType::BinaryAndHash),
        _ => match value.trim().parse::<i32>() {
            Ok(0) => Ok(DataBlockIndexType::BinarySearch),
            Ok(1) => Ok(DataBlockIndexType::BinaryAndHash),
            Ok(other) => Err(format!(
                "Invalid data_block_index_type value in RocksDBFactory backend: {other}"
            )),
            Err(error) => Err(format!(
                "Invalid data_block_index_type value in RocksDBFactory backend: {value}. Error: {error}"
            )),
        },
    }
}

fn parse_block_based_index_type(value: &str) -> Result<BlockBasedIndexType, String> {
    let normalized = value
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "");
    match normalized.as_str() {
        "binarysearch" | "binary" => Ok(BlockBasedIndexType::BinarySearch),
        "hashsearch" | "hash" => Ok(BlockBasedIndexType::HashSearch),
        "twolevelindexsearch" | "twolevel" => Ok(BlockBasedIndexType::TwoLevelIndexSearch),
        _ => match value.trim().parse::<i32>() {
            Ok(0) => Ok(BlockBasedIndexType::BinarySearch),
            Ok(1) => Ok(BlockBasedIndexType::HashSearch),
            Ok(2) => Ok(BlockBasedIndexType::TwoLevelIndexSearch),
            Ok(other) => Err(format!(
                "Invalid index_type value in RocksDBFactory backend: {other}"
            )),
            Err(error) => Err(format!(
                "Invalid index_type value in RocksDBFactory backend: {value}. Error: {error}"
            )),
        },
    }
}

fn parse_db_compression_type(value: &str) -> Result<DBCompressionType, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "knoocompression" => Ok(DBCompressionType::None),
        "snappy" | "ksnappycompression" => Ok(DBCompressionType::Snappy),
        "zlib" | "kzlibcompression" => Ok(DBCompressionType::Zlib),
        "bz2" | "kbz2compression" => Ok(DBCompressionType::Bz2),
        "lz4" | "klz4compression" => Ok(DBCompressionType::Lz4),
        "lz4hc" | "klz4hccompression" => Ok(DBCompressionType::Lz4hc),
        "zstd" | "kzstdcompression" => Ok(DBCompressionType::Zstd),
        other => Err(format!(
            "Invalid compression value in RocksDBFactory backend: {other}"
        )),
    }
}

fn parse_db_compaction_style(value: &str) -> Result<DBCompactionStyle, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "level" | "kcompactionstylelevel" => Ok(DBCompactionStyle::Level),
        "universal" | "kcompactionstyleuniversal" => Ok(DBCompactionStyle::Universal),
        "fifo" | "kcompactionstylefifo" => Ok(DBCompactionStyle::Fifo),
        other => Err(format!(
            "Invalid compaction_style value in RocksDBFactory backend: {other}"
        )),
    }
}

fn parse_checksum_type(value: &str) -> Result<ChecksumType, String> {
    let normalized = value
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-', ' '], "");
    match normalized.as_str() {
        "none" | "nochecksum" => Ok(ChecksumType::NoChecksum),
        "crc32c" | "crc32" => Ok(ChecksumType::CRC32c),
        "xxhash" => Ok(ChecksumType::XXHash),
        "xxhash64" => Ok(ChecksumType::XXHash64),
        "xxh3" => Ok(ChecksumType::XXH3),
        _ => match value.trim().parse::<i32>() {
            Ok(0) => Ok(ChecksumType::NoChecksum),
            Ok(1) => Ok(ChecksumType::CRC32c),
            Ok(2) => Ok(ChecksumType::XXHash),
            Ok(3) => Ok(ChecksumType::XXHash64),
            Ok(4) => Ok(ChecksumType::XXH3),
            Ok(other) => Err(format!(
                "Invalid checksum_type value in RocksDBFactory backend: {other}"
            )),
            Err(error) => Err(format!(
                "Invalid checksum_type value in RocksDBFactory backend: {value}. Error: {error}"
            )),
        },
    }
}

fn status_code_from_error_kind(kind: ErrorKind) -> Option<i32> {
    let code = match kind {
        ErrorKind::NotFound => 1,
        ErrorKind::Corruption => 2,
        ErrorKind::NotSupported => 3,
        ErrorKind::InvalidArgument => 4,
        ErrorKind::IOError => 5,
        ErrorKind::MergeInProgress => 6,
        ErrorKind::Incomplete => 7,
        ErrorKind::ShutdownInProgress => 8,
        ErrorKind::TimedOut => 9,
        ErrorKind::Aborted => 10,
        ErrorKind::Busy => 11,
        ErrorKind::Expired => 12,
        ErrorKind::TryAgain => 13,
        ErrorKind::CompactionTooLarge => 14,
        ErrorKind::ColumnFamilyDropped => 15,
        ErrorKind::Unknown => return None,
    };
    Some(100 + code)
}

pub(crate) fn rocksdb_error_kind_to_status(kind: ErrorKind) -> Status {
    status_code_from_error_kind(kind)
        .map(Status::custom_code)
        .unwrap_or(Status::BackendError)
}

fn rocksdb_error_to_status(error: &rocksdb::Error) -> Status {
    rocksdb_error_kind_to_status(error.kind())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CppStyleDbOptionOverrides {
    disable_auto_compactions: Option<bool>,
    bytes_per_sync: Option<u64>,
    wal_bytes_per_sync: Option<u64>,
    max_open_files: Option<i32>,
    write_buffer_size: Option<usize>,
    target_file_size_base: Option<u64>,
    target_file_size_multiplier: Option<i32>,
    max_bytes_for_level_base: Option<u64>,
    max_background_flushes: Option<i32>,
    compression: Option<DBCompressionType>,
    compaction_style: Option<DBCompactionStyle>,
}

fn parse_cpp_style_db_option_overrides(
    raw_value: &str,
) -> Result<CppStyleDbOptionOverrides, String> {
    let pairs = parse_db_option_pairs(raw_value)?;
    if pairs.is_empty() {
        return Ok(CppStyleDbOptionOverrides::default());
    }

    let filtered_pairs: Vec<_> = pairs
        .into_iter()
        .filter(|(key, _)| !key.trim().eq_ignore_ascii_case("block_based_table_factory"))
        .collect();
    if filtered_pairs.is_empty() {
        return Ok(CppStyleDbOptionOverrides::default());
    }

    let mut overrides = CppStyleDbOptionOverrides::default();
    for (key, value) in filtered_pairs {
        let normalized = key.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "disable_auto_compactions" => {
                overrides.disable_auto_compactions = Some(parse_bool_like(&value)?);
            }
            "bytes_per_sync" => {
                overrides.bytes_per_sync = Some(value.parse::<u64>().map_err(|error| {
                    format!(
                        "Invalid bytes_per_sync value in RocksDBFactory backend: {value}. Error: {error}"
                    )
                })?);
            }
            "wal_bytes_per_sync" => {
                overrides.wal_bytes_per_sync = Some(value.parse::<u64>().map_err(|error| {
                    format!(
                        "Invalid wal_bytes_per_sync value in RocksDBFactory backend: {value}. Error: {error}"
                    )
                })?);
            }
            "max_open_files" => {
                overrides.max_open_files = Some(value.parse::<i32>().map_err(|error| {
                    format!(
                        "Invalid max_open_files value in RocksDBFactory backend: {value}. Error: {error}"
                    )
                })?);
            }
            "write_buffer_size" => {
                overrides.write_buffer_size = Some(parse_size_like(&value)?);
            }
            "target_file_size_base" => {
                overrides.target_file_size_base = Some(parse_size_like(&value)? as u64);
            }
            "target_file_size_multiplier" => {
                overrides.target_file_size_multiplier =
                    Some(value.parse::<i32>().map_err(|error| {
                        format!(
                            "Invalid target_file_size_multiplier value in RocksDBFactory backend: {value}. Error: {error}"
                        )
                    })?);
            }
            "max_bytes_for_level_base" => {
                overrides.max_bytes_for_level_base = Some(parse_size_like(&value)? as u64);
            }
            "max_background_flushes" => {
                overrides.max_background_flushes = Some(value.parse::<i32>().map_err(|error| {
                    format!(
                        "Invalid max_background_flushes value in RocksDBFactory backend: {value}. Error: {error}"
                    )
                })?);
            }
            "compression" => {
                overrides.compression = Some(parse_db_compression_type(&value)?);
            }
            "compaction_style" => {
                overrides.compaction_style = Some(parse_db_compaction_style(&value)?);
            }
            _ => {}
        }
    }

    Ok(overrides)
}

fn should_apply_runtime_option(key: &str) -> bool {
    !matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "block_based_table_factory"
            | "disable_auto_compactions"
            | "bytes_per_sync"
            | "wal_bytes_per_sync"
            | "max_open_files"
            | "write_buffer_size"
            | "target_file_size_base"
            | "target_file_size_multiplier"
            | "max_bytes_for_level_base"
            | "max_background_flushes"
            | "compression"
            | "compaction_style"
    )
}

fn format_option<T>(name: &str, value: Option<T>) -> Option<String>
where
    T: std::fmt::Display,
{
    value.map(|value| format!("{name}={value}"))
}

fn summarize_db_options(config: &RocksDbConfigSnapshot) -> String {
    let mut parts = vec![
        format!("create_if_missing=true"),
        format!("compression={:?}", config.compression),
        format!("path={}", config.path),
        format!("hard_set={}", config.hard_set),
    ];

    parts.extend(
        [
            format_option("max_open_files", config.max_open_files),
            format_option("fd_required", Some(config.fd_required)),
            format_option("target_file_size_base", config.target_file_size_base),
            format_option("max_bytes_for_level_base", config.max_bytes_for_level_base),
            format_option(
                "write_buffer_size",
                config.write_buffer_size.map(|value| value as u64),
            ),
            format_option(
                "target_file_size_multiplier",
                config.target_file_size_multiplier,
            ),
            format_option("bg_threads", config.bg_threads),
            format_option("high_threads", config.high_threads),
            format_option("max_background_flushes", config.max_background_flushes),
            format_option(
                "universal_compaction",
                Some(config.universal_compaction as i32),
            ),
            format_option(
                "min_write_buffer_number_to_merge",
                config.min_write_buffer_number_to_merge,
            ),
            format_option("max_write_buffer_number", config.max_write_buffer_number),
            format_option("options", config.options.as_deref()),
            format_option("bbt_options", config.bbt_options.as_deref()),
        ]
        .into_iter()
        .flatten(),
    );

    parts.join("; ")
}

fn summarize_cf_options(config: &RocksDbConfigSnapshot) -> String {
    let mut parts = vec![
        format!("block_size={:?}", config.block_size),
        format!("cache_mb={:?}", config.cache_mb),
        format!("filter_bits={:?}", config.filter_bits),
        format!("filter_full={}", config.filter_full),
    ];
    parts.extend(
        [
            format_option(
                "block_size_bytes",
                config.block_size.map(|value| value as u64),
            ),
            format_option("partition_filters", None::<i32>),
            format_option("data_block_index_type", None::<i32>),
            format_option("data_block_hash_ratio", None::<f64>),
            format_option("index_type", None::<i32>),
        ]
        .into_iter()
        .flatten(),
    );

    parts.join("; ")
}

fn apply_block_based_option_pairs(
    block_options: &mut BlockBasedOptions,
    raw_value: &str,
) -> Result<(), String> {
    for (key, value) in parse_db_option_pairs(raw_value)? {
        let normalized = key.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "block_based_table_factory" => {
                apply_block_based_option_pairs(block_options, &value)?;
            }
            "block_size" => block_options.set_block_size(parse_size_like(&value)?),
            "metadata_block_size" => {
                block_options.set_metadata_block_size(parse_size_like(&value)?)
            }
            "partition_filters" => block_options.set_partition_filters(parse_bool_like(&value)?),
            "cache_index_and_filter_blocks" => {
                block_options.set_cache_index_and_filter_blocks(parse_bool_like(&value)?);
            }
            "pin_l0_filter_and_index_blocks_in_cache" => {
                block_options.set_pin_l0_filter_and_index_blocks_in_cache(parse_bool_like(&value)?);
            }
            "pin_top_level_index_and_filter" => {
                block_options.set_pin_top_level_index_and_filter(parse_bool_like(&value)?);
            }
            "format_version" => {
                block_options.set_format_version(value.trim().parse::<i32>().map_err(|error| {
                    format!(
                        "Invalid format_version value in RocksDBFactory backend: {value}. Error: {error}"
                    )
                })?);
            }
            "block_restart_interval" => {
                block_options.set_block_restart_interval(value.trim().parse::<i32>().map_err(
                    |error| {
                        format!(
                            "Invalid block_restart_interval value in RocksDBFactory backend: {value}. Error: {error}"
                        )
                    },
                )?);
            }
            "index_block_restart_interval" => {
                block_options.set_index_block_restart_interval(
                    value.trim().parse::<i32>().map_err(|error| {
                        format!(
                            "Invalid index_block_restart_interval value in RocksDBFactory backend: {value}. Error: {error}"
                        )
                    })?,
                );
            }
            "data_block_index_type" => {
                block_options.set_data_block_index_type(parse_data_block_index_type(&value)?);
            }
            "data_block_hash_ratio" => {
                block_options.set_data_block_hash_ratio(value.trim().parse::<f64>().map_err(
                    |error| {
                        format!(
                            "Invalid data_block_hash_ratio value in RocksDBFactory backend: {value}. Error: {error}"
                        )
                    },
                )?);
            }
            "whole_key_filtering" => {
                block_options.set_whole_key_filtering(parse_bool_like(&value)?);
            }
            "optimize_filters_for_memory" => {
                block_options.set_optimize_filters_for_memory(parse_bool_like(&value)?);
            }
            "block_cache" => {
                let cache = Cache::new_lru_cache(parse_size_like(&value)?);
                block_options.set_block_cache(&cache);
            }
            "no_block_cache" => {
                if parse_bool_like(&value)? {
                    block_options.disable_cache();
                }
            }
            "index_type" => block_options.set_index_type(parse_block_based_index_type(&value)?),
            "checksum" | "checksum_type" => {
                block_options.set_checksum_type(parse_checksum_type(&value)?)
            }
            "filter_policy" => {
                let normalized = value
                    .trim()
                    .to_ascii_lowercase()
                    .replace(['_', '-', ' '], "");
                match normalized.as_str() {
                    "bloom" | "bloomfilter" | "fullbloom" | "fullbloomfilter" => {
                        block_options.set_bloom_filter(10.0, true);
                    }
                    "full" | "fullfilter" => {
                        block_options.set_bloom_filter(10.0, false);
                    }
                    "ribbon" => block_options.set_ribbon_filter(10.0),
                    "hybridribbon" | "hybridribbonfilter" => {
                        block_options.set_hybrid_ribbon_filter(10.0, 2)
                    }
                    "none" => {}
                    _ => {
                        return Err(format!(
                            "Unsupported filter_policy value in RocksDBFactory backend: {value}"
                        ));
                    }
                }
            }
            _ => {
                return Err(format!(
                    "Unsupported bbt_options key in RocksDBFactory backend: {key}"
                ));
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RocksDbConfigSnapshot {
    pub path: String,
    pub hard_set: bool,
    pub cache_mb: Option<usize>,
    pub filter_bits: Option<i32>,
    pub filter_full: bool,
    pub max_open_files: Option<i32>,
    pub fd_required: i32,
    pub target_file_size_base: Option<u64>,
    pub max_bytes_for_level_base: Option<u64>,
    pub write_buffer_size: Option<usize>,
    pub target_file_size_multiplier: Option<i32>,
    pub bg_threads: Option<i32>,
    pub high_threads: Option<i32>,
    pub max_background_flushes: Option<i32>,
    pub block_size: Option<usize>,
    pub universal_compaction: bool,
    pub min_write_buffer_number_to_merge: Option<i32>,
    pub max_write_buffer_number: Option<i32>,
    pub compression: DBCompressionType,
    pub options: Option<String>,
    pub bbt_options: Option<String>,
}

impl RocksDbConfigSnapshot {
    pub fn from_section(section: &Section) -> Result<Self, String> {
        let path = read_string(section, "path");
        if path.is_empty() {
            return Err("Missing path in RocksDBFactory backend".to_owned());
        }

        let hard_set = read_optional_bool_like(section, "hard_set")?.unwrap_or(false);

        let cache_mb = read_optional_parsed::<i32>(section, "cache_mb")?.map(|value| {
            let value = value as usize;
            if !hard_set && value == 256 {
                1024
            } else {
                value
            }
        });

        let max_open_files = read_optional_parsed::<i32>(section, "open_files")?.map(|value| {
            if !hard_set && value == 2000 {
                8000
            } else {
                value
            }
        });
        let fd_required = max_open_files.map_or(2048, |value| value + 128);

        let target_file_size_base =
            read_optional_parsed::<i32>(section, "file_size_mb")?.map(|value| {
                let value = if !hard_set && value == 8 { 256 } else { value };
                megabytes(value as usize) as u64
            });
        let max_bytes_for_level_base = target_file_size_base.map(|value| value * 5);
        let mut write_buffer_size = target_file_size_base.map(|value| (value as usize) * 2);

        let universal_compaction =
            read_optional_parsed::<i32>(section, "universal_compaction")?.unwrap_or(0) != 0;
        let min_write_buffer_number_to_merge = universal_compaction.then_some(2);
        let max_write_buffer_number = universal_compaction.then_some(6);
        if universal_compaction {
            write_buffer_size = target_file_size_base.map(|value| (value as usize) * 6);
        }

        let high_threads = read_optional_parsed::<i32>(section, "high_threads")?;

        Ok(Self {
            path,
            hard_set,
            cache_mb,
            filter_bits: read_optional_parsed::<i32>(section, "filter_bits")?,
            filter_full: read_optional_parsed::<i32>(section, "filter_full")?.unwrap_or(0) != 0,
            max_open_files,
            fd_required,
            target_file_size_base,
            max_bytes_for_level_base,
            write_buffer_size,
            target_file_size_multiplier: read_optional_parsed::<i32>(section, "file_size_mult")?,
            bg_threads: read_optional_parsed::<i32>(section, "bg_threads")?,
            high_threads,
            max_background_flushes: high_threads.filter(|threads| *threads > 0),
            block_size: read_optional_parsed::<i32>(section, "block_size")?
                .map(|value| value as usize),
            universal_compaction,
            min_write_buffer_number_to_merge,
            max_write_buffer_number,
            compression: DBCompressionType::Snappy,
            options: read_optional_string(section, "options")?,
            bbt_options: read_optional_string(section, "bbt_options")?,
        })
    }
}

pub struct RocksDbBackend {
    key_bytes: usize,
    journal: Arc<dyn NodeStoreJournal>,
    config: RocksDbConfigSnapshot,
    delete_path: AtomicBool,
    database: Arc<Mutex<Option<DBWithThreadMode<MultiThreaded>>>>,
    batch_writer: Arc<BatchWriter>,
}

impl RocksDbBackend {
    pub fn new(
        key_bytes: usize,
        parameters: &Section,
        scheduler: Arc<dyn Scheduler>,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> Result<Self, String> {
        let config = RocksDbConfigSnapshot::from_section(parameters)?;
        journal.log(
            JournalLevel::Debug,
            &format!("RocksDB DBOptions: {}", summarize_db_options(&config)),
        );
        journal.log(
            JournalLevel::Debug,
            &format!("RocksDB CFOptions: {}", summarize_cf_options(&config)),
        );
        let database = Arc::new(Mutex::new(None));
        let batch_writer = BatchWriter::new(
            {
                let database = Arc::clone(&database);
                let journal = Arc::clone(&journal);
                move |batch| write_batch_to_database(&database, key_bytes, batch, &journal)
            },
            Arc::clone(&scheduler),
        );

        Ok(Self {
            key_bytes,
            journal,
            config,
            delete_path: AtomicBool::new(false),
            database,
            batch_writer,
        })
    }

    pub fn config_snapshot(&self) -> &RocksDbConfigSnapshot {
        &self.config
    }

    fn open_database(&self) -> std::sync::MutexGuard<'_, Option<DBWithThreadMode<MultiThreaded>>> {
        self.database
            .lock()
            .expect("rocksdb database mutex must not be poisoned")
    }

    fn apply_runtime_options(&self, db: &DBWithThreadMode<MultiThreaded>) -> Result<(), String> {
        let Some(raw_options) = self.config.options.as_deref() else {
            return Ok(());
        };

        let pairs: Vec<_> = parse_db_option_pairs(raw_options)?
            .into_iter()
            .filter(|(key, _)| should_apply_runtime_option(key))
            .collect();
        if pairs.is_empty() {
            return Ok(());
        }

        let runtime_pairs: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();

        db.set_options(&runtime_pairs)
            .map_err(|error| error.to_string())
    }

    fn build_options(&self, create_if_missing: bool) -> Result<Options, String> {
        let mut options = Options::default();
        options.create_if_missing(create_if_missing);
        options.set_compression_type(self.config.compression);

        let mut env = Env::new().map_err(|error| error.to_string())?;
        if let Some(bg_threads) = self.config.bg_threads {
            env.set_background_threads(bg_threads);
        }
        if let Some(high_threads) = self.config.high_threads {
            env.set_high_priority_background_threads(high_threads);
        }
        options.set_env(&env);

        if let Some(open_files) = self.config.max_open_files {
            options.set_max_open_files(open_files);
        }
        if let Some(target) = self.config.target_file_size_base {
            options.set_target_file_size_base(target);
        }
        if let Some(max_bytes) = self.config.max_bytes_for_level_base {
            options.set_max_bytes_for_level_base(max_bytes);
        }
        if let Some(write_buffer_size) = self.config.write_buffer_size {
            options.set_write_buffer_size(write_buffer_size);
        }
        if let Some(multiplier) = self.config.target_file_size_multiplier {
            options.set_target_file_size_multiplier(multiplier);
        }
        if let Some(flushes) = self.config.max_background_flushes {
            #[allow(deprecated)]
            options.set_max_background_flushes(flushes);
        }
        if self.config.universal_compaction {
            options.set_compaction_style(DBCompactionStyle::Universal);
        }
        if let Some(min_merge) = self.config.min_write_buffer_number_to_merge {
            options.set_min_write_buffer_number_to_merge(min_merge);
        }
        if let Some(max_buffers) = self.config.max_write_buffer_number {
            options.set_max_write_buffer_number(max_buffers);
        }
        if let Some(raw_options) = self.config.options.as_deref() {
            let overrides = parse_cpp_style_db_option_overrides(raw_options)?;
            if let Some(disable_auto_compactions) = overrides.disable_auto_compactions {
                options.set_disable_auto_compactions(disable_auto_compactions);
            }
            if let Some(bytes_per_sync) = overrides.bytes_per_sync {
                options.set_bytes_per_sync(bytes_per_sync);
            }
            if let Some(max_open_files) = overrides.max_open_files {
                options.set_max_open_files(max_open_files);
            }
            if let Some(write_buffer_size) = overrides.write_buffer_size {
                options.set_write_buffer_size(write_buffer_size);
            }
            if let Some(target_file_size_base) = overrides.target_file_size_base {
                options.set_target_file_size_base(target_file_size_base);
            }
            if let Some(target_file_size_multiplier) = overrides.target_file_size_multiplier {
                options.set_target_file_size_multiplier(target_file_size_multiplier);
            }
            if let Some(max_bytes_for_level_base) = overrides.max_bytes_for_level_base {
                options.set_max_bytes_for_level_base(max_bytes_for_level_base);
            }
            if let Some(max_background_flushes) = overrides.max_background_flushes {
                #[allow(deprecated)]
                options.set_max_background_flushes(max_background_flushes);
            }
            if let Some(compression) = overrides.compression {
                options.set_compression_type(compression);
            }
            if let Some(compaction_style) = overrides.compaction_style {
                options.set_compaction_style(compaction_style);
            }
        }

        let mut block_options = BlockBasedOptions::default();
        let mut cache_guard = None;
        if let Some(cache_mb) = self.config.cache_mb {
            let cache = Cache::new_lru_cache(megabytes(cache_mb));
            block_options.set_block_cache(&cache);
            cache_guard = Some(cache);
        }
        if let Some(filter_bits) = self.config.filter_bits {
            let filter_blocks = !self.config.filter_full;
            block_options.set_bloom_filter(filter_bits as f64, filter_blocks);
        }
        if let Some(block_size) = self.config.block_size {
            block_options.set_block_size(block_size);
        }
        if let Some(raw_options) = self.config.options.as_deref() {
            for (key, value) in parse_db_option_pairs(raw_options)? {
                if key.trim().eq_ignore_ascii_case("block_based_table_factory") {
                    apply_block_based_option_pairs(&mut block_options, &value)?;
                }
            }
        }
        if let Some(raw_bbt_options) = self.config.bbt_options.as_deref() {
            apply_block_based_option_pairs(&mut block_options, raw_bbt_options)?;
        }
        options.set_block_based_table_factory(&block_options);
        let _ = cache_guard;

        Ok(options)
    }

    fn decode_object(&self, hash: &Uint256, value: &[u8]) -> (Option<Arc<NodeObject>>, Status) {
        let decoded = DecodedBlob::new(hash.data(), value);
        if decoded.was_ok() {
            (Some(decoded.create_object()), Status::Ok)
        } else {
            (None, Status::DataCorrupt)
        }
    }
}

impl Backend for RocksDbBackend {
    fn get_name(&self) -> String {
        self.config.path.clone()
    }

    fn get_block_size(&self) -> Option<usize> {
        self.config.block_size
    }

    fn open(&self, create_if_missing: bool) -> Result<(), String> {
        let mut database = self.open_database();
        if database.is_some() {
            return Err("database is already open".to_owned());
        }

        let options = self.build_options(create_if_missing)?;
        let db = DBWithThreadMode::<MultiThreaded>::open(&options, &self.config.path)
            .map_err(|error| format!("Unable to open/create RocksDB: {error}"))?;
        self.apply_runtime_options(&db)?;
        *database = Some(db);
        Ok(())
    }

    fn open_deterministic(
        &self,
        create_if_missing: bool,
        _app_type: u64,
        _uid: u64,
        _salt: u64,
    ) -> Result<(), String> {
        self.open(create_if_missing)
    }

    fn is_open(&self) -> bool {
        self.database
            .lock()
            .expect("rocksdb database mutex must not be poisoned")
            .is_some()
    }

    fn close(&self) -> Result<(), String> {
        self.batch_writer.wait_for_writing();

        let mut database = self.open_database();
        database.take();
        drop(database);

        if self.delete_path.load(Ordering::Relaxed) {
            let path = Path::new(&self.config.path);
            if path.exists() {
                fs::remove_dir_all(path).map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    fn fetch(&self, hash: &Uint256) -> (Option<Arc<NodeObject>>, Status) {
        let database = self.open_database();
        let db = database
            .as_ref()
            .expect("xrpl::NodeStore::RocksDBBackend::fetch : non-null database");

        match db.get(hash.data()) {
            Ok(Some(value)) => self.decode_object(hash, &value),
            Ok(None) => (None, Status::NotFound),
            Err(error) => {
                self.journal.log(JournalLevel::Error, error.as_ref());
                (None, rocksdb_error_to_status(&error))
            }
        }
    }

    fn fetch_batch(&self, hashes: &[Uint256]) -> (Vec<Option<Arc<NodeObject>>>, Status) {
        let database = self.open_database();
        let db = database
            .as_ref()
            .expect("xrpl::NodeStore::RocksDBBackend::fetch : non-null database");

        let keys = hashes.iter().map(|hash| hash.data());
        let raw_results = db.multi_get(keys);
        let mut results = Vec::with_capacity(hashes.len());
        for (hash, raw_result) in hashes.iter().zip(raw_results) {
            match raw_result {
                Ok(Some(value)) => {
                    let (object, status) = self.decode_object(hash, &value);
                    if status == Status::Ok {
                        results.push(object);
                    } else {
                        results.push(None);
                    }
                }
                Ok(None) => results.push(None),
                Err(error) => {
                    self.journal.log(JournalLevel::Error, error.as_ref());
                    results.push(None);
                }
            }
        }
        (results, Status::Ok)
    }

    fn store(&self, object: Arc<NodeObject>) {
        self.batch_writer.store(object);
    }

    fn store_batch(&self, batch: &crate::Batch) {
        write_batch_to_database(&self.database, self.key_bytes, batch, &self.journal);
    }

    fn sync(&self) {}

    fn for_each(&self, callback: &mut dyn FnMut(Arc<NodeObject>)) {
        let database = self.open_database();
        let db = database
            .as_ref()
            .expect("xrpl::NodeStore::RocksDBBackend::for_each : non-null database");

        for entry in db.iterator(IteratorMode::Start) {
            match entry {
                Ok((key, value)) => {
                    if key.len() != self.key_bytes {
                        self.journal.log(
                            JournalLevel::Fatal,
                            &format!("Bad key size = {}", key.len()),
                        );
                        continue;
                    }

                    let hash = match Uint256::from_slice(&key) {
                        Some(hash) => hash,
                        None => {
                            self.journal
                                .log(JournalLevel::Fatal, "Bad key size = invalid hash bytes");
                            continue;
                        }
                    };

                    let (object, status) = self.decode_object(&hash, &value);
                    if status == Status::Ok {
                        if let Some(object) = object {
                            callback(object);
                        }
                    } else {
                        self.journal
                            .log(JournalLevel::Fatal, &format!("Corrupt NodeObject #{hash}"));
                    }
                }
                Err(error) => {
                    self.journal.log(JournalLevel::Fatal, error.as_ref());
                }
            }
        }
    }

    fn get_write_load(&self) -> i32 {
        self.batch_writer.get_write_load()
    }

    fn set_delete_path(&self) {
        self.delete_path.store(true, Ordering::Relaxed);
    }

    fn fd_required(&self) -> i32 {
        self.config.fd_required
    }
}

impl Drop for RocksDbBackend {
    fn drop(&mut self) {
        let _ = <Self as Backend>::close(self);
    }
}

fn write_batch_to_database(
    database: &Arc<Mutex<Option<DBWithThreadMode<MultiThreaded>>>>,
    key_bytes: usize,
    batch: &crate::Batch,
    journal: &Arc<dyn NodeStoreJournal>,
) {
    let database = database
        .lock()
        .expect("rocksdb database mutex must not be poisoned");
    let db = database
        .as_ref()
        .expect("xrpl::NodeStore::RocksDBBackend::storeBatch : non-null database");

    let mut write_batch = WriteBatch::default();
    for object in batch {
        let encoded = EncodedBlob::new(object);
        write_batch.put(&encoded.get_key()[..key_bytes], encoded.get_data());
    }

    if let Err(error) = db.write(write_batch) {
        journal.log(JournalLevel::Error, error.as_ref());
        panic!("storeBatch failed: {error}");
    }
}

pub struct RocksDbFactory;

impl RocksDbFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RocksDbFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl Factory for RocksDbFactory {
    fn get_name(&self) -> String {
        "RocksDB".to_owned()
    }

    fn create_instance(
        &self,
        key_bytes: usize,
        parameters: &Section,
        _burst_size: usize,
        scheduler: Arc<dyn Scheduler>,
        journal: Arc<dyn NodeStoreJournal>,
    ) -> crate::factory::BackendResult {
        Ok(Box::new(RocksDbBackend::new(
            key_bytes, parameters, scheduler, journal,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::basic_config::Section;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn base_section(path: &str) -> Section {
        let mut section = Section::new("node_db");
        section.set("type", "RocksDB");
        section.set("path", path);
        section
    }

    #[test]
    fn status_custom_code_round_trips_the_numeric_value() {
        let status = Status::custom_code(115);
        assert_eq!(status.code(), Some(115));
        assert_eq!(Status::BackendError.code(), None);
    }

    #[test]
    fn rocksdb_error_kind_uses_cpp_status_code_numbers_when_available() {
        assert_eq!(
            rocksdb_error_kind_to_status(ErrorKind::NotFound),
            Status::custom_code(101)
        );
        assert_eq!(
            rocksdb_error_kind_to_status(ErrorKind::Corruption),
            Status::custom_code(102)
        );
        assert_eq!(
            rocksdb_error_kind_to_status(ErrorKind::ColumnFamilyDropped),
            Status::custom_code(115)
        );
        assert_eq!(
            rocksdb_error_kind_to_status(ErrorKind::Unknown),
            Status::BackendError
        );
    }

    #[test]
    fn rocksdb_block_based_options_accept_cpp_style_keys() {
        let dir = TempDir::new().expect("tempdir");
        let mut section = base_section(&dir.path().join("rocksdb-options").to_string_lossy());
        section.set(
            "bbt_options",
            "block_size=4096;index_type=hash_search;checksum_type=xxh3;filter_policy=hybrid_ribbon;no_block_cache=1",
        );

        let backend = RocksDbBackend::new(
            NodeObject::KEY_BYTES,
            &section,
            Arc::new(crate::DummyScheduler),
            Arc::new(crate::NullJournal),
        )
        .expect("backend");

        backend.build_options(true).expect("build options");
    }

    #[test]
    fn rocksdb_uses_cpp_parser_for_preopen_option_strings() {
        let dir = TempDir::new().expect("tempdir");
        let mut section = base_section(&dir.path().join("rocksdb-cpp-parser").to_string_lossy());
        section.set(
            "options",
            "disable_auto_compactions=true;compaction_style=universal;block_based_table_factory={block_size=4096;index_type=hash_search}",
        );

        let backend = RocksDbBackend::new(
            NodeObject::KEY_BYTES,
            &section,
            Arc::new(crate::DummyScheduler),
            Arc::new(crate::NullJournal),
        )
        .expect("backend");

        backend.build_options(true).expect("build options");
    }
}
