use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

use dashmap::DashMap;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use tauri_plugin_log::log;

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
    let cache: DashMap<String, VerificationEntry> = DashMap::new();
    for (k, v) in serializable.files {
        cache.insert(k, v);
    }

    // Size cap: if cache is excessively large, clear it entirely
    const MAX_CACHE_ENTRIES: usize = 50_000;
    if cache.len() > MAX_CACHE_ENTRIES {
        log::warn!(
            "Verification cache has {} entries (max {}), clearing",
            cache.len(),
            MAX_CACHE_ENTRIES
        );
        cache.clear();
        return cache;
    }

    // Prune stale entries where the file no longer exists
    let before = cache.len();
    cache.retain(|rel_path, _| {
        let full_path = game_dir.join(rel_path);
        full_path.exists()
    });
    let removed = before - cache.len();
    if removed > 0 {
        log::info!(
            "Pruned {}/{} stale entries from verification cache",
            removed,
            before
        );
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
    game_dir: &Path,
    cache: &DashMap<String, VerificationEntry>,
) -> io::Result<bool> {
    let cache_key = path
        .strip_prefix(game_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
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

    if let Some(entry) = cache.get(&cache_key)
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
            cache_key,
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
    use std::sync::Arc;
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
        // Create actual files so load_verification_cache doesn't prune them as stale
        let game_dir = dir.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        let f1 = game_dir.join("file1");
        let f2 = game_dir.join("file2");
        let f3 = game_dir.join("file3");
        fs::write(&f1, b"a").unwrap();
        fs::write(&f2, b"bb").unwrap();
        fs::write(&f3, b"ccc").unwrap();
        let cache = DashMap::new();
        cache.insert(
            "file1".to_string(),
            VerificationEntry {
                size: 100,
                md5: "abc123".to_string(),
                mtime_secs: 1000,
            },
        );
        cache.insert(
            "file2".to_string(),
            VerificationEntry {
                size: 200,
                md5: "def456".to_string(),
                mtime_secs: 2000,
            },
        );
        cache.insert(
            "file3".to_string(),
            VerificationEntry {
                size: 300,
                md5: "ghi789".to_string(),
                mtime_secs: 3000,
            },
        );
        save_verification_cache(&game_dir, &cache).unwrap();
        let loaded = load_verification_cache(&game_dir);
        assert_eq!(loaded.len(), 3);
        let e1 = loaded.get("file1").unwrap();
        assert_eq!(e1.size, 100);
        assert_eq!(e1.md5, "abc123");
        assert_eq!(e1.mtime_secs, 1000);
        let e2 = loaded.get("file2").unwrap();
        assert_eq!(e2.size, 200);
        assert_eq!(e2.md5, "def456");
        let e3 = loaded.get("file3").unwrap();
        assert_eq!(e3.size, 300);
        assert_eq!(e3.md5, "ghi789");
    }

    #[test]
    fn check_file_md5_cached_miss_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DashMap::new();
        let missing = dir.path().join("nonexistent.dat");
        let result = check_file_md5_cached(&missing, 10, "abc", dir.path(), &cache).unwrap();
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
        let rel_path = file_path
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .to_string();
        cache.insert(
            rel_path,
            VerificationEntry {
                size: 11,
                md5: md5.clone(),
                mtime_secs: mtime,
            },
        );
        let result = check_file_md5_cached(&file_path, 11, &md5, dir.path(), &cache).unwrap();
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
        let rel_path = file_path
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .to_string();
        cache.insert(
            rel_path,
            VerificationEntry {
                size: 11,
                md5: "old_md5".to_string(),
                mtime_secs: mtime,
            },
        );
        let result = check_file_md5_cached(&file_path, 20, "old_md5", dir.path(), &cache).unwrap();
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
        let rel_path = file_path
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .to_string();
        cache.insert(
            rel_path,
            VerificationEntry {
                size: 11,
                md5: "stale_cached_md5".to_string(),
                mtime_secs: mtime,
            },
        );
        let wrong_expected = "wrong_expected_md5";
        assert_ne!(wrong_expected, &actual_md5);
        let result =
            check_file_md5_cached(&file_path, 11, wrong_expected, dir.path(), &cache).unwrap();
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
        let result = check_file_md5_cached(&file_path, 11, &md5, dir.path(), &cache).unwrap();
        assert!(result);
        assert_eq!(cache.len(), 1);
        let rel_path = file_path
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .to_string();
        let entry = cache.get(&rel_path).unwrap();
        assert_eq!(entry.size, 11);
        assert_eq!(entry.md5, md5);
        assert_eq!(entry.mtime_secs, mtime);
    }

    #[test]
    fn save_verification_cache_atomic_write() {
        let dir = tempfile::tempdir().unwrap();
        let game_dir = dir.path().join("game");
        fs::create_dir_all(&game_dir).unwrap();
        // Create the file on disk so load_verification_cache doesn't prune it
        fs::write(game_dir.join("file"), b"x").unwrap();
        let cache = DashMap::new();
        cache.insert(
            "file".to_string(),
            VerificationEntry {
                size: 42,
                md5: "deadbeef".to_string(),
                mtime_secs: 999,
            },
        );
        let tmp_path = game_dir.join(format!("{}.tmp", VERIFICATION_CACHE_FILE));
        assert!(!tmp_path.exists());
        save_verification_cache(&game_dir, &cache).unwrap();
        assert!(!tmp_path.exists());
        let cache_path = game_dir.join(VERIFICATION_CACHE_FILE);
        assert!(cache_path.exists());
        let loaded = load_verification_cache(&game_dir);
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn concurrent_cache_access() {
        let cache: Arc<DashMap<String, VerificationEntry>> = Arc::new(DashMap::new());
        let mut handles = Vec::new();
        for i in 0..8 {
            let c = Arc::clone(&cache);
            handles.push(std::thread::spawn(move || {
                for j in 0..100 {
                    let key = format!("key-{}-{}", i, j);
                    c.insert(
                        key.clone(),
                        VerificationEntry {
                            size: i as u64 + j as u64,
                            md5: format!("md5-{}-{}", i, j),
                            mtime_secs: i as u64 * 100 + j as u64,
                        },
                    );
                    if let Some(entry) = c.get(&key) {
                        assert_eq!(entry.size, i as u64 + j as u64);
                        assert_eq!(entry.md5, format!("md5-{}-{}", i, j));
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(cache.len(), 800);
    }

    #[test]
    fn check_file_md5_cached_with_zero_size_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.dat");
        fs::write(&file_path, b"").unwrap();
        let cache = DashMap::new();
        let md5 = {
            let mut hasher = Md5::new();
            hasher.update(b"");
            hex::encode(hasher.finalize())
        };
        let result = check_file_md5_cached(&file_path, 0, &md5, dir.path(), &cache).unwrap();
        assert!(result);
    }
}
