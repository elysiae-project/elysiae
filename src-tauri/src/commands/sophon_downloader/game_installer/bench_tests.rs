//! Micro-benchmarks for key optimized code paths in the Sophon game installer.
//!
//! Run with: cargo test --lib -- --nocapture bench_

use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::api::{is_known_vo_locale, vo_lang_matches};

// ---------------------------------------------------------------------------
// 1. vo_lang_matches
// ---------------------------------------------------------------------------
#[test]
fn bench_vo_lang_matches() {
    let prefixes = [
        "Audio/en-us/PKG",
        "Audio/zh-cn/PKG",
        "Audio/ja-jp/PKG",
        "Audio/ko-kr/PKG",
        "Audio/zh-tw/PKG",
    ];
    let langs = ["cn", "en", "jp", "kr"];

    let n = 100_000;
    let fields: Vec<String> = (0..n)
        .map(|i| {
            let p = &prefixes[i % prefixes.len()];
            format!("{p}/chunk_{i:06}.pak")
        })
        .collect();

    // Warm up
    for _ in 0..1000 {
        let _ = vo_lang_matches(&fields[0], langs[0]);
    }

    let start = Instant::now();
    for (i, field) in fields.iter().enumerate() {
        let lang = langs[i % langs.len()];
        std::hint::black_box(vo_lang_matches(field, lang));
    }
    let elapsed = start.elapsed();

    let total_ns = elapsed.as_nanos() as f64;
    let per_call_ns = total_ns / n as f64;
    let throughput = n as f64 / elapsed.as_secs_f64();

    println!("bench_vo_lang_matches:");
    println!("  total:   {elapsed:?}");
    println!("  per-call: {per_call_ns:.1} ns");
    println!("  throughput: {throughput:.0} calls/sec");
}

// ---------------------------------------------------------------------------
// 2. is_known_vo_locale
// ---------------------------------------------------------------------------
#[test]
fn bench_is_known_vo_locale() {
    let known = [
        "en-us".to_string(),
        "zh-cn".to_string(),
        "zh-tw".to_string(),
        "ko-kr".to_string(),
        "ja-jp".to_string(),
    ];
    let unknown = [
        "game".to_string(),
        "cutscenes".to_string(),
        "Audio".to_string(),
        "CG".to_string(),
        "data".to_string(),
    ];

    let n = 100_000;
    let fields: Vec<String> = (0..n)
        .map(|i| {
            if i % 2 == 0 {
                known[i % known.len()].clone()
            } else {
                unknown[i % unknown.len()].clone()
            }
        })
        .collect();

    // Warm up
    for _ in 0..1000 {
        let _ = is_known_vo_locale(&fields[0]);
    }

    let start = Instant::now();
    for field in &fields {
        std::hint::black_box(is_known_vo_locale(field));
    }
    let elapsed = start.elapsed();

    let total_ns = elapsed.as_nanos() as f64;
    let per_call_ns = total_ns / n as f64;
    let throughput = n as f64 / elapsed.as_secs_f64();

    println!("bench_is_known_vo_locale:");
    println!("  total:   {elapsed:?}");
    println!("  per-call: {per_call_ns:.1} ns");
    println!("  throughput: {throughput:.0} calls/sec");
}

// ---------------------------------------------------------------------------
// 3. Pre-computed lowercase filter vs. per-call .to_lowercase()
// ---------------------------------------------------------------------------
#[test]
fn bench_precomputed_lowercase_filter() {
    let blacklist: Vec<String> = [
        "starrail_data/persistent/audio",
        "starrail_data/streamingassets/audio",
        "starrail_data/persistent/video",
        "starrail_data/streamingassets/video",
        "starrail_data/persistent/block",
        "starrail_data/streamingassets/block",
        "hkrpg_data/persistent/audio",
        "hkrpg_data/streamingassets/audio",
        "hkrpg_data/persistent/video",
        "hkrpg_data/streamingassets/video",
        "hkrpg_data/persistent/block",
        "hkrpg_data/streamingassets/block",
        "Audio/EN-US/PKG",
        "Audio/ZH-CN/PKG",
        "Audio/JA-JP/PKG",
        "Audio/KO-KR/PKG",
        "Cutscenes/CG_Anim",
        "VideoAssets/Cutscene",
        "DownloadBlacklist/Entry",
        "StreamingAssets/Data",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let n = 10_000;
    let asset_names: Vec<String> = (0..n)
        .map(|i| {
            if i % 5 == 0 {
                // ~20% match — should be filtered
                format!("StarRail_Data/Persistent/Audio/voice_{i:05}.pck")
            } else if i % 5 == 1 {
                format!("StarRail_Data/StreamingAssets/Audio/sfx_{i:05}.pak")
            } else {
                format!("StarRail_Data/AssetBundles/scenes/level_{i:05}.unity")
            }
        })
        .collect();

    // --- NEW: pre-computed lowercase blacklist ---
    let blacklist_lower: Vec<String> = blacklist.iter().map(|e| e.to_lowercase()).collect();

    let start = Instant::now();
    let mut filtered_new = 0u64;
    for asset in &asset_names {
        let asset_lower = asset.to_lowercase();
        for entry in &blacklist_lower {
            if asset_lower.contains(entry) {
                filtered_new += 1;
                break;
            }
        }
    }
    let elapsed_new = start.elapsed();

    // --- OLD: .to_lowercase() on every blacklist entry per comparison ---
    let start = Instant::now();
    let mut filtered_old = 0u64;
    for asset in &asset_names {
        let asset_lower = asset.to_lowercase();
        for entry in &blacklist {
            if asset_lower.contains(&entry.to_lowercase()) {
                filtered_old += 1;
                break;
            }
        }
    }
    let elapsed_old = start.elapsed();

    assert_eq!(
        filtered_new, filtered_old,
        "both approaches must filter the same count"
    );

    let ratio = elapsed_old.as_nanos() as f64 / elapsed_new.as_nanos().max(1) as f64;

    println!("bench_precomputed_lowercase_filter:");
    println!("  pre-computed: {elapsed_new:?}  (filtered {filtered_new})");
    println!("  per-call:     {elapsed_old:?}  (filtered {filtered_old})");
    println!("  speedup:      {ratio:.2}x");
}

// ---------------------------------------------------------------------------
// 4. try_lock vs lock
// ---------------------------------------------------------------------------
#[test]
fn bench_try_lock_vs_lock() {
    let n = 1_000_000;
    let mutex: Mutex<Instant> = Mutex::new(Instant::now());

    // --- try_lock path ---
    let start = Instant::now();
    for _ in 0..n {
        if let Ok(guard) = mutex.try_lock() {
            let _ = guard.elapsed();
            drop(guard);
        }
    }
    let elapsed_try = start.elapsed();

    // --- lock().unwrap() path ---
    let start = Instant::now();
    for _ in 0..n {
        let guard = mutex.lock().unwrap();
        let _ = guard.elapsed();
        drop(guard);
    }
    let elapsed_lock = start.elapsed();

    let ratio = elapsed_lock.as_nanos() as f64 / elapsed_try.as_nanos().max(1) as f64;

    println!("bench_try_lock_vs_lock:");
    println!("  try_lock: {elapsed_try:?}  ({n} iterations)");
    println!("  lock:     {elapsed_lock:?}  ({n} iterations)");
    println!("  speedup:  {ratio:.2}x (try_lock / lock)");
}

// ---------------------------------------------------------------------------
// 5. Arc::clone vs Vec::clone (deep clone)
// ---------------------------------------------------------------------------
#[test]
fn bench_arc_clone_vs_vec_clone() {
    let n = 10_000;
    let data: Vec<String> = (0..10_000).map(|i| format!("string_value_{i}")).collect();
    let arc: Arc<Vec<String>> = Arc::new(data);

    // --- Arc::clone (shallow, just refcount) ---
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box(Arc::clone(&arc));
    }
    let elapsed_arc = start.elapsed();

    // --- Deep clone via (*arc).clone() ---
    let start = Instant::now();
    for _ in 0..n {
        std::hint::black_box((*arc).clone());
    }
    let elapsed_vec = start.elapsed();

    let ratio = elapsed_vec.as_nanos() as f64 / elapsed_arc.as_nanos().max(1) as f64;

    println!("bench_arc_clone_vs_vec_clone:");
    println!("  Arc::clone:     {elapsed_arc:?}  ({n} iterations)");
    println!("  Vec::clone:     {elapsed_vec:?}  ({n} iterations)");
    println!("  speedup:        {ratio:.2}x (Arc clone is cheaper)");
}

// ---------------------------------------------------------------------------
// 6. set_len (sparse) vs writing zeros
// ---------------------------------------------------------------------------
#[test]
fn bench_set_len_vs_zero_fill() {
    let file_count = 10;
    let file_size: u64 = 10 * 1024 * 1024; // 10 MiB
    let zero_buf = vec![0u8; file_size as usize];

    // --- set_len (sparse allocation) ---
    let dir_sparse = tempfile::tempdir().expect("tempdir for sparse");
    let start = Instant::now();
    for i in 0..file_count {
        let path = dir_sparse.path().join(format!("sparse_{i}.dat"));
        let file = std::fs::File::create(&path).expect("create file");
        file.set_len(file_size).expect("set_len");
        drop(file);
    }
    let elapsed_set_len = start.elapsed();

    // --- write zeros ---
    let dir_zeros = tempfile::tempdir().expect("tempdir for zeros");
    let start = Instant::now();
    for i in 0..file_count {
        let path = dir_zeros.path().join(format!("zeros_{i}.dat"));
        let mut file = std::fs::File::create(&path).expect("create file");
        std::io::Write::write_all(&mut file, &zero_buf).expect("write zeros");
        drop(file);
    }
    let elapsed_zero_fill = start.elapsed();

    let ratio = elapsed_zero_fill.as_nanos() as f64 / elapsed_set_len.as_nanos().max(1) as f64;

    println!("bench_set_len_vs_zero_fill:");
    println!(
        "  set_len (sparse): {elapsed_set_len:?}  ({file_count} x {} MiB)",
        file_size / (1024 * 1024)
    );
    println!(
        "  zero-fill:        {elapsed_zero_fill:?}  ({file_count} x {} MiB)",
        file_size / (1024 * 1024)
    );
    println!("  speedup:          {ratio:.2}x (set_len is cheaper)");
}
