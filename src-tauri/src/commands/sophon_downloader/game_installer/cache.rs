use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

use dashmap::DashMap;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

use super::{MD5_HASH_BUFFER_SIZE, VERIFICATION_CACHE_FILE};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationEntry {
    pub size: u64,
    pub md5: String,
    pub mtime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VerificationCacheSerializable {
    files: HashMap<String, VerificationEntry>,
}

pub fn load_verification_cache(game_dir: &Path) -> DashMap<String, VerificationEntry> {
    let cache_path = game_dir.join(VERIFICATION_CACHE_FILE);
    let serializable: VerificationCacheSerializable = match File::open(&cache_path) {
        Ok(f) => serde_json::from_reader(f).unwrap_or_else(|_| VerificationCacheSerializable {
            files: HashMap::new(),
        }),
        Err(_) => VerificationCacheSerializable {
            files: HashMap::new(),
        },
    };
    let cache = DashMap::new();
    for (k, v) in serializable.files {
        cache.insert(k, v);
    }
    cache
}

pub fn save_verification_cache(
    game_dir: &Path,
    cache: &DashMap<String, VerificationEntry>,
) -> io::Result<()> {
    let cache_path = game_dir.join(VERIFICATION_CACHE_FILE);
    let tmp_path = cache_path.with_extension("tmp");
    let serializable = VerificationCacheSerializable {
        files: cache
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect(),
    };
    {
        let f = File::create(&tmp_path)?;
        serde_json::to_writer(f, &serializable)?;
    }
    fs::rename(&tmp_path, &cache_path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp_path);
    })?;
    Ok(())
}

pub fn check_file_md5_cached(
    path: &Path,
    expected_size: u64,
    expected_md5: &str,
    cache: &DashMap<String, VerificationEntry>,
) -> io::Result<bool> {
    let path_str = path.to_string_lossy().to_string();
    let metadata = match path.metadata() {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let mtime = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Some(entry) = cache.get(&path_str)
        && entry.size == expected_size
        && entry.md5 == expected_md5
        && entry.mtime_secs == mtime
    {
        return Ok(true);
    }

    if metadata.len() != expected_size {
        return Ok(false);
    }

    let actual = file_md5_hex(path)?;
    let matches = actual == expected_md5;

    if matches {
        cache.insert(
            path_str,
            VerificationEntry {
                size: expected_size,
                md5: expected_md5.to_string(),
                mtime_secs: mtime,
            },
        );
    }

    Ok(matches)
}

fn file_md5_hex(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Md5::new();
    let mut buf = [0u8; MD5_HASH_BUFFER_SIZE];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}
