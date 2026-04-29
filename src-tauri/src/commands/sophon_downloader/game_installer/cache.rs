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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::UNIX_EPOCH;

    #[test]
    fn load_verification_cache_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = load_verification_cache(dir.path());
        assert!(cache.is_empty());
    }

    #[test]
    fn load_verification_cache_corrupted_json() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join(VERIFICATION_CACHE_FILE);
        fs::write(&cache_path, "this is not json!!!").unwrap();
        let cache = load_verification_cache(dir.path());
        assert!(cache.is_empty());
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DashMap::new();
        cache.insert(
            "/path/to/file1".to_string(),
            VerificationEntry {
                size: 100,
                md5: "abc123".to_string(),
                mtime_secs: 1000,
            },
        );
        cache.insert(
            "/path/to/file2".to_string(),
            VerificationEntry {
                size: 200,
                md5: "def456".to_string(),
                mtime_secs: 2000,
            },
        );
        cache.insert(
            "/path/to/file3".to_string(),
            VerificationEntry {
                size: 300,
                md5: "ghi789".to_string(),
                mtime_secs: 3000,
            },
        );
        save_verification_cache(dir.path(), &cache).unwrap();
        let loaded = load_verification_cache(dir.path());
        assert_eq!(loaded.len(), 3);
        let e1 = loaded.get("/path/to/file1").unwrap();
        assert_eq!(e1.size, 100);
        assert_eq!(e1.md5, "abc123");
        assert_eq!(e1.mtime_secs, 1000);
        let e2 = loaded.get("/path/to/file2").unwrap();
        assert_eq!(e2.size, 200);
        assert_eq!(e2.md5, "def456");
        let e3 = loaded.get("/path/to/file3").unwrap();
        assert_eq!(e3.size, 300);
        assert_eq!(e3.md5, "ghi789");
    }

    #[test]
    fn check_file_md5_cached_miss_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DashMap::new();
        let missing = dir.path().join("nonexistent.dat");
        let result = check_file_md5_cached(&missing, 10, "abc", &cache).unwrap();
        assert!(!result);
    }

    #[test]
    fn check_file_md5_cached_hit() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        fs::write(&file_path, b"hello world").unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let cache = DashMap::new();
        let md5 = {
            let mut hasher = Md5::new();
            hasher.update(b"hello world");
            hex::encode(hasher.finalize())
        };
        cache.insert(
            file_path.to_string_lossy().to_string(),
            VerificationEntry {
                size: 11,
                md5: md5.clone(),
                mtime_secs: mtime,
            },
        );
        let result = check_file_md5_cached(&file_path, 11, &md5, &cache).unwrap();
        assert!(result);
    }

    #[test]
    fn check_file_md5_cached_miss_size_changed() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        fs::write(&file_path, b"hello world").unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let cache = DashMap::new();
        cache.insert(
            file_path.to_string_lossy().to_string(),
            VerificationEntry {
                size: 11,
                md5: "old_md5".to_string(),
                mtime_secs: mtime,
            },
        );
        let result = check_file_md5_cached(&file_path, 20, "old_md5", &cache).unwrap();
        assert!(!result);
    }

    #[test]
    fn check_file_md5_cached_miss_md5_changed() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        fs::write(&file_path, b"hello world").unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let cache = DashMap::new();
        let actual_md5 = {
            let mut hasher = Md5::new();
            hasher.update(b"hello world");
            hex::encode(hasher.finalize())
        };
        cache.insert(
            file_path.to_string_lossy().to_string(),
            VerificationEntry {
                size: 11,
                md5: "stale_cached_md5".to_string(),
                mtime_secs: mtime,
            },
        );
        let wrong_expected = "wrong_expected_md5";
        assert_ne!(wrong_expected, &actual_md5);
        let result = check_file_md5_cached(&file_path, 11, wrong_expected, &cache).unwrap();
        assert!(!result);
    }

    #[test]
    fn check_file_md5_cached_populates_on_match() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.dat");
        fs::write(&file_path, b"hello world").unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        let mtime = metadata
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let cache = DashMap::new();
        let md5 = {
            let mut hasher = Md5::new();
            hasher.update(b"hello world");
            hex::encode(hasher.finalize())
        };
        assert!(cache.is_empty());
        let result = check_file_md5_cached(&file_path, 11, &md5, &cache).unwrap();
        assert!(result);
        assert_eq!(cache.len(), 1);
        let entry = cache.get(&file_path.to_string_lossy().to_string()).unwrap();
        assert_eq!(entry.size, 11);
        assert_eq!(entry.md5, md5);
        assert_eq!(entry.mtime_secs, mtime);
    }
}
