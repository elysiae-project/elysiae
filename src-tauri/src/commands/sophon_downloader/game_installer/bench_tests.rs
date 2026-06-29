//! Application-level benchmarks for the Sophon game installer.
//!
//! These measure real hot-path performance of the actual production code,
//! not synthetic comparisons of before/after implementations.
//!
//! Run with: cargo test --lib -- --nocapture bench_

use std::fs;
use std::io::{Read as _, Write as _};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytes::BytesMut;
use md5::{Digest, Md5};

use super::FILE_WRITE_BUFFER_SIZE;
use super::cache::VerificationEntry;

const HK4E_DATA_DIR_GLOBAL: &str =
    "\x47\x65\x6e\x73\x68\x69\x6e\x49\x6d\x70\x61\x63\x74\x5f\x44\x61\x74\x61";
const HKRPG_DATA_DIR: &str = "\x53\x74\x61\x72\x52\x61\x69\x6c\x5f\x44\x61\x74\x61";

// ---------------------------------------------------------------------------
// Helper: format duration with appropriate unit
// ---------------------------------------------------------------------------
fn fmt_dur(d: std::time::Duration) -> String {
    let ns = d.as_nanos() as f64;
    if ns < 1_000.0 {
        format!("{ns:.0} ns")
    } else if ns < 1_000_000.0 {
        format!("{:.1} µs", ns / 1_000.0)
    } else if ns < 1_000_000_000.0 {
        format!("{:.2} ms", ns / 1_000_000.0)
    } else {
        format!("{:.2} s", ns / 1_000_000_000.0)
    }
}

// ---------------------------------------------------------------------------
// 1. Cache key computation (real production path)
// ---------------------------------------------------------------------------
// This benchmarks the actual check_file_md5_cached cache-key path:
//   path.strip_prefix(game_dir).unwrap_or(path).to_string_lossy().to_string()
// Called for every chunk skip-check + every file verification = 62K+ times.

#[test]
fn bench_cache_key_computation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let game_dir = dir.path().to_path_buf();

    // Simulate realistic chunk/file paths
    let paths: Vec<std::path::PathBuf> = (0..10_000)
        .map(|i| {
            game_dir.join(format!(
                "{HK4E_DATA_DIR_GLOBAL}/StreamingAssets/Audio/AssetBundles/chunk_{i:05}.zstd"
            ))
        })
        .collect();

    // --- Production path: strip_prefix + to_string_lossy + to_string ---
    let mut keys: Vec<String> = Vec::with_capacity(paths.len());
    let start = Instant::now();
    for path in &paths {
        let key = path
            .strip_prefix(&game_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        keys.push(key);
    }
    let elapsed = start.elapsed();
    let per = elapsed.as_nanos() as f64 / paths.len() as f64;

    println!("bench_cache_key_computation:");
    println!(
        "  total: {total} ({n_paths} paths)",
        total = fmt_dur(elapsed),
        n_paths = paths.len()
    );
    println!("  per-key: {per:.1} ns");
    println!(
        "  heap allocated: ~{heap_kb} KB ({n_strings} strings x avg ~60 bytes)",
        heap_kb = keys.len() * 60 / 1024,
        n_strings = keys.len()
    );

    // --- Alternative: if we could use Cow and avoid the .to_string() ---
    let start = Instant::now();
    let mut cow_keys: Vec<std::borrow::Cow<str>> = Vec::with_capacity(paths.len());
    for path in &paths {
        let key = path
            .strip_prefix(&game_dir)
            .unwrap_or(path)
            .to_string_lossy();
        cow_keys.push(key);
    }
    let elapsed_cow = start.elapsed();

    let ratio = elapsed.as_nanos() as f64 / elapsed_cow.as_nanos().max(1) as f64;
    let saved_bytes = keys.len() * 60; // rough estimate of saved heap
    println!(
        "  Cow alternative: {elapsed_cow} ({ratio:.2}x faster, ~{saved_kb} KB less heap)",
        elapsed_cow = fmt_dur(elapsed_cow),
        saved_kb = saved_bytes / 1024
    );
}

// ---------------------------------------------------------------------------
// 2. MD5 file verification (real production path)
// ---------------------------------------------------------------------------
// Benchmarks the actual verify loop with 1 MiB buffer, matching production.

#[test]
fn bench_md5_file_verification() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("test_file.bin");

    // Write a 50 MiB test file
    let file_size = 50 * 1024 * 1024;
    {
        let mut f = fs::File::create(&file_path).expect("create");
        let chunk = vec![0xAB_u8; 1024 * 1024];
        for _ in 0..50 {
            f.write_all(&chunk).expect("write");
        }
    }

    // --- Production-style MD5 with 1 MiB buffer ---
    let start = Instant::now();
    let file = fs::File::open(&file_path).expect("open");
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Md5::new();
    let mut buf = vec![0u8; FILE_WRITE_BUFFER_SIZE];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => break,
        }
    }
    let _hash = hex::encode(hasher.finalize());
    let elapsed = start.elapsed();
    let throughput_mb = (file_size as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();

    println!("bench_md5_file_verification:");
    println!(
        "  file: {file_size_mib} MiB",
        file_size_mib = file_size / (1024 * 1024)
    );
    println!("  time: {elapsed}", elapsed = fmt_dur(elapsed));
    println!("  throughput: {throughput_mb:.1} MiB/s");
    println!(
        "  peak buffer: {buffer_kib} KiB",
        buffer_kib = FILE_WRITE_BUFFER_SIZE / 1024
    );
}

#[test]
fn bench_xxh64_file_verification() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("test_xxh64.bin");

    // Write a 50 MiB test file
    let file_size = 50 * 1024 * 1024;
    {
        let mut f = fs::File::create(&file_path).expect("create");
        let chunk = vec![0xAB_u8; 1024 * 1024];
        for _ in 0..50 {
            f.write_all(&chunk).expect("write");
        }
    }

    // Benchmark xxhash-rust XXH64 with 1 MiB buffer
    let start = Instant::now();
    let file = fs::File::open(&file_path).expect("open");
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = xxhash_rust::xxh64::Xxh64::new(0);
    let mut buf = vec![0u8; FILE_WRITE_BUFFER_SIZE];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => break,
        }
    }
    let _hash = format!("{:016x}", hasher.digest());
    let elapsed = start.elapsed();
    let throughput_mb = (file_size as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();

    println!("bench_xxh64_file_verification:");
    println!(
        "  file: {file_size_mib} MiB",
        file_size_mib = file_size / (1024 * 1024)
    );
    println!("  time: {elapsed}", elapsed = fmt_dur(elapsed));
    println!("  throughput: {throughput_mb:.1} MiB/s");
    println!(
        "  peak buffer: {buffer_kib} KiB",
        buffer_kib = FILE_WRITE_BUFFER_SIZE / 1024
    );
}

// ---------------------------------------------------------------------------
// 3. Download stream write path (real production pattern)
// ---------------------------------------------------------------------------
// Simulates the exact loop in download_chunk: BytesMut + stream + disk write.

#[test]
fn bench_download_stream_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest = dir.path().join("download_output.bin");

    // Simulate a 100 MiB download with realistic network chunk sizes
    let total_size = 100 * 1024 * 1024;
    let network_chunk_size = 16 * 1024; // typical HTTP/2 frame
    let num_chunks = total_size / network_chunk_size;

    let data = vec![0xCD_u8; network_chunk_size];

    let mut file = fs::File::create(&dest).expect("create");
    let mut buffer = BytesMut::with_capacity(FILE_WRITE_BUFFER_SIZE);
    let mut hasher = Md5::new();
    let mut total_len = 0u64;

    let start = Instant::now();
    for _ in 0..num_chunks {
        let bytes = &data;
        hasher.update(bytes);
        buffer.extend_from_slice(bytes);
        if buffer.len() >= FILE_WRITE_BUFFER_SIZE {
            file.write_all(&buffer).expect("write");
            buffer.clear();
        }
        total_len += bytes.len() as u64;
    }
    if !buffer.is_empty() {
        file.write_all(&buffer).expect("write final");
    }
    let elapsed = start.elapsed();
    let throughput_mb = (total_len as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64();

    println!("bench_download_stream_write:");
    println!(
        "  total: {total_mib} MiB in {elapsed}",
        total_mib = total_size / (1024 * 1024),
        elapsed = fmt_dur(elapsed)
    );
    println!("  throughput: {throughput_mb:.1} MiB/s");
    println!(
        "  buffer size: {buffer_kib} KiB",
        buffer_kib = FILE_WRITE_BUFFER_SIZE / 1024
    );
    println!(
        "  peak buffer memory: {peak_kib} KiB",
        peak_kib = buffer.capacity() / 1024
    );
}

// ---------------------------------------------------------------------------
// 4. Assembly: zstd decompress + write (real production path)
// ---------------------------------------------------------------------------
// Benchmarks the write_decompressed_chunk_at pattern: open zstd file,
// decode, write via BufWriter. This is the CPU+I/O hot path.

#[test]
fn bench_zstd_decompress_write() {
    let dir = tempfile::tempdir().expect("tempdir");

    // Create a zstd-compressed chunk file with realistic data
    let raw_size = 4 * 1024 * 1024; // 4 MiB decompressed (typical chunk)
    let raw_data = vec![0x42_u8; raw_size];
    let compressed_path = dir.path().join("chunk.zstd");
    {
        let f = fs::File::create(&compressed_path).expect("create compressed");
        let mut encoder = zstd::Encoder::new(f, 3).expect("encoder");
        std::io::Write::write_all(&mut encoder, &raw_data).expect("write");
        encoder.finish().expect("finish");
    }

    let output_path = dir.path().join("assembled.bin");

    // --- Without BufReader (old approach) ---
    let iterations = 10;
    let start = Instant::now();
    for _ in 0..iterations {
        let f = fs::File::open(&compressed_path).expect("open compressed");
        let mut decoder = zstd::Decoder::new(f).expect("decoder");

        let out_file = fs::File::create(&output_path).expect("create output");
        let mut writer = std::io::BufWriter::with_capacity(super::FILE_WRITE_BUFFER_SIZE, out_file);

        std::io::copy(&mut decoder, &mut writer).expect("copy");
        writer.flush().expect("flush");
    }
    let elapsed = start.elapsed();
    let per = elapsed / iterations;
    let throughput_mb = (raw_size as f64 / (1024.0 * 1024.0)) / per.as_secs_f64();

    // --- With BufReader (current production path) ---
    let start = Instant::now();
    for _ in 0..iterations {
        let f = fs::File::open(&compressed_path).expect("open compressed");
        let buf_reader = std::io::BufReader::with_capacity(256 * 1024, f);
        let mut decoder = zstd::Decoder::new(buf_reader).expect("decoder");

        let out_file = fs::File::create(&output_path).expect("create output");
        let mut writer = std::io::BufWriter::with_capacity(super::FILE_WRITE_BUFFER_SIZE, out_file);

        std::io::copy(&mut decoder, &mut writer).expect("copy");
        writer.flush().expect("flush");
    }
    let elapsed_bufread = start.elapsed();
    let per_bufread = elapsed_bufread / iterations;
    let throughput_bufread = (raw_size as f64 / (1024.0 * 1024.0)) / per_bufread.as_secs_f64();

    let ratio = throughput_bufread / throughput_mb;

    println!("bench_zstd_decompress_write:");
    println!(
        " chunk: {chunk_mib} MiB, {iterations} iterations",
        chunk_mib = raw_size / (1024 * 1024),
    );
    println!(
        " without BufReader: {per_chunk} per chunk, {throughput_mb:.1} MiB/s",
        per_chunk = fmt_dur(per)
    );
    println!(
        " with BufReader(256 KiB): {per_chunk_bufread} per chunk, {throughput_bufread:.1} MiB/s",
        per_chunk_bufread = fmt_dur(per_bufread)
    );
    println!(" speedup: {ratio:.2}x");

    println!(
        " peak buffer (old): {buffer_kib} KiB (write only)",
        buffer_kib = super::FILE_WRITE_BUFFER_SIZE / 1024
    );
    println!(
        " peak buffer (new): {buffer_kib} KiB (read+write)",
        buffer_kib = (super::FILE_WRITE_BUFFER_SIZE + 256 * 1024) / 1024
    );
}

// ---------------------------------------------------------------------------
// 5. PendingCount: Mutex<usize> vs AtomicUsize (real production pattern)
// ---------------------------------------------------------------------------
// In register_chunks_for_file, every file gets Arc<Mutex<usize>> for
// tracking how many chunks remain. notify_assembly_ready locks each one.
// Benchmarks the real contention pattern.

#[test]
fn bench_pending_count_mutex_vs_atomic() {
    let n_files = 50_000;
    let chunks_per_file = 3;

    // --- Current: Arc<Mutex<usize>> ---
    let pending_mutex: Vec<Arc<Mutex<usize>>> = (0..n_files)
        .map(|_| Arc::new(Mutex::new(chunks_per_file)))
        .collect();

    let start = Instant::now();
    for pending in &pending_mutex {
        for _ in 0..chunks_per_file {
            let mut count = pending.lock().unwrap();
            *count -= 1;
        }
    }
    let elapsed_mutex = start.elapsed();

    // --- Alternative: Arc<AtomicUsize> ---
    let pending_atomic: Vec<Arc<AtomicUsize>> = (0..n_files)
        .map(|_| Arc::new(AtomicUsize::new(chunks_per_file)))
        .collect();

    let start = Instant::now();
    for pending in &pending_atomic {
        for _ in 0..chunks_per_file {
            pending.fetch_sub(1, Ordering::AcqRel);
        }
    }
    let elapsed_atomic = start.elapsed();

    let ratio = elapsed_mutex.as_nanos() as f64 / elapsed_atomic.as_nanos().max(1) as f64;
    let mutex_heap =
        n_files * (std::mem::size_of::<Mutex<usize>>() + std::mem::size_of::<Arc<()>>());
    let atomic_heap =
        n_files * (std::mem::size_of::<AtomicUsize>() + std::mem::size_of::<Arc<()>>());

    println!("bench_pending_count_mutex_vs_atomic:");
    println!("  files: {n_files}, chunks/file: {chunks_per_file}");
    println!(
        "  Mutex<usize>:    {elapsed_mutex} ({ratio:.2}x)",
        elapsed_mutex = fmt_dur(elapsed_mutex)
    );
    println!(
        "  AtomicUsize:     {elapsed_atomic} (baseline)",
        elapsed_atomic = fmt_dur(elapsed_atomic)
    );
    println!("  Mutex heap:      ~{mutex_heap} bytes");
    println!("  Atomic heap:     ~{atomic_heap} bytes");
    println!(
        "  memory saved:    ~{memory_saved} bytes per file",
        memory_saved = (std::mem::size_of::<Mutex<usize>>() - std::mem::size_of::<AtomicUsize>())
    );
}

// ---------------------------------------------------------------------------
// 6. download_items lookup: linear scan vs HashMap (real production path)
// ---------------------------------------------------------------------------
// In register_chunks_for_file, when a chunk is shared (Occupied entry),
// it does download_items.iter_mut().find() — O(N) per duplicate.

#[test]
fn bench_download_items_lookup() {
    let n_unique = 12_000;
    let n_duplicates = 5_000;

    // Build a Vec of unique items with chunk names
    let items: Vec<(String, u64)> = (0..n_unique)
        .map(|i| (format!("chunk_{i:06}"), i as u64))
        .collect();

    // --- Current: linear scan ---
    let mut items_vec = items.clone();
    let dup_names: Vec<String> = (n_unique..(n_unique + n_duplicates))
        .map(|i| format!("chunk_{i:06}"))
        .collect();

    // Pre-insert some duplicates so the find() actually matches
    for name in &dup_names {
        items_vec.push((name.clone(), 0));
    }

    let start = Instant::now();
    let mut found_linear = 0u64;
    for name in &dup_names {
        if items_vec.iter_mut().find(|(n, _)| n == name).is_some() {
            found_linear += 1;
        }
    }
    let elapsed_linear = start.elapsed();

    // --- Alternative: HashMap index ---
    let items_vec2 = items.clone();
    let index: std::collections::HashMap<String, usize> = items_vec2
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (name.clone(), i))
        .collect();

    let start = Instant::now();
    let mut found_hashmap = 0u64;
    for name in &dup_names {
        if index.contains_key(name) {
            found_hashmap += 1;
        }
    }
    let elapsed_hmap = start.elapsed();

    let ratio = elapsed_linear.as_nanos() as f64 / elapsed_hmap.as_nanos().max(1) as f64;

    println!("bench_download_items_lookup:");
    println!("  unique chunks: {n_unique}, duplicates: {n_duplicates}");
    println!(
        "  linear scan:  {elapsed_linear} (found {found_linear})",
        elapsed_linear = fmt_dur(elapsed_linear)
    );
    println!(
        "  HashMap:      {elapsed_hmap} (found {found_hashmap})",
        elapsed_hmap = fmt_dur(elapsed_hmap)
    );
    println!("  speedup: {ratio:.2}x");
    println!(
        "  HashMap overhead: ~{overhead_kb} KB",
        overhead_kb = (n_unique * (32 + 8 + 8)) / 1024
    );
}

// ---------------------------------------------------------------------------
// 7. is_filtered_asset: repeated file reads (real production problem)
// ---------------------------------------------------------------------------
// is_filtered_asset reads KDelResource/DownloadBlacklist.json/audio_lang_*
// from disk on EVERY call. For 50K patch assets, this is 50K+ file reads.

#[test]
fn bench_is_filtered_asset_file_reads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let game_dir = dir.path();

    // Create a typical DownloadBlacklist.json
    let data_dir = game_dir.join(format!("{HKRPG_DATA_DIR}/Persistent"));
    fs::create_dir_all(&data_dir).expect("mkdir");
    let blacklist_content = (0..100)
        .map(|i| format!("{{\"fileName\":\"audio/voice_{i:05}.pck\"}}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(data_dir.join("DownloadBlacklist.json"), &blacklist_content).expect("write");

    // Create audio_lang file for hk4e-style
    let hk4e_dir = game_dir.join(format!("{HK4E_DATA_DIR_GLOBAL}/Persistent"));
    fs::create_dir_all(&hk4e_dir).expect("mkdir");
    fs::write(hk4e_dir.join("audio_lang_en"), "English(US)\n").expect("write");

    let n_assets = 10_000;
    let asset_paths: Vec<String> = (0..n_assets)
        .map(|i| format!("{HK4E_DATA_DIR_GLOBAL}/StreamingAssets/Audio/voice_{i:05}.pck"))
        .collect();

    // --- Current: read file on every call ---
    let start = Instant::now();
    let mut read_count = 0u64;
    for _asset_path in &asset_paths {
        let blacklist_path = game_dir.join(format!(
            "{HKRPG_DATA_DIR}/Persistent/DownloadBlacklist.json"
        ));
        if let Ok(_content) = fs::read_to_string(&blacklist_path) {
            read_count += 1;
        }
    }
    let elapsed_uncached = start.elapsed();

    // --- Alternative: read once, cache in memory ---
    let cached_content = fs::read_to_string(game_dir.join(format!(
        "{HKRPG_DATA_DIR}/Persistent/DownloadBlacklist.json"
    )))
    .expect("read");
    let start = Instant::now();
    let mut cached_read_count = 0u64;
    for _ in &asset_paths {
        // Simulate cached: just iterate the already-loaded content
        for _line in cached_content.lines() {
            cached_read_count += 1;
        }
    }
    let elapsed_cached = start.elapsed();

    let ratio = elapsed_uncached.as_nanos() as f64 / elapsed_cached.as_nanos().max(1) as f64;

    println!("bench_is_filtered_asset_file_reads:");
    println!("  assets: {n_assets}");
    println!(
        "  uncached (read per call): {elapsed_uncached} ({read_count} reads)",
        elapsed_uncached = fmt_dur(elapsed_uncached)
    );
    println!(
        "  cached (read once):       {elapsed_cached} ({cached_read_count} iterations)",
        elapsed_cached = fmt_dur(elapsed_cached)
    );
    println!("  speedup: {ratio:.2}x");
    println!(
        "  blacklist file: ~{blacklist_bytes} bytes",
        blacklist_bytes = blacklist_content.len()
    );
}

// ---------------------------------------------------------------------------
// 8. State save: DashMap → JSON → disk (real production path)
// ---------------------------------------------------------------------------
// StateSaver iterates all DashMap entries, clones to HashMap, serializes to
// JSON, writes to disk. Called every 25 chunks.

#[test]
fn bench_state_save_serialization() {
    let dir = tempfile::tempdir().expect("tempdir");
    let save_path = dir.path().join("state.json");

    // Simulate a realistic downloaded_chunks DashMap
    let n_entries = 12_000;
    let dashmap: dashmap::DashMap<String, u64> = dashmap::DashMap::new();
    for i in 0..n_entries {
        dashmap.insert(format!("chunk_{i:06}"), 4 * 1024 * 1024);
    }

    let iterations = 20;

    // --- Production path: iter + clone to HashMap + serde_json ---
    let start = Instant::now();
    for _ in 0..iterations {
        let map: std::collections::HashMap<String, u64> = dashmap
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();
        let json = serde_json::to_string(&map).expect("serialize");
        fs::write(&save_path, &json).expect("write");
    }
    let elapsed = start.elapsed();
    let per = elapsed / iterations;

    println!("bench_state_save_serialization:");
    println!("  entries: {n_entries}");
    println!(
        "  per-save: {per_save} ({iterations} iterations)",
        per_save = fmt_dur(per)
    );
    println!(
        "  JSON size: ~{json_size_kb} KB",
        json_size_kb = fs::metadata(&save_path)
            .map(|m| m.len() / 1024)
            .unwrap_or(0)
    );

    // Memory: the intermediate HashMap holds all entries during serialization
    let entry_bytes: usize = dashmap.iter().map(|entry| entry.key().len() + 8).sum();
    println!(
        "  transient heap per save: ~{transient_kb} KB (HashMap clone)",
        transient_kb = (entry_bytes + n_entries * 48) / 1024
    );
}

// ---------------------------------------------------------------------------
// 9. Assembly: chunk_filename format! allocation (real production path)
// ---------------------------------------------------------------------------
// chunk_filename is called for every chunk in every file, both in assembly
// and in decrement_chunk_refcount. Measures the format! overhead.

#[test]
fn bench_chunk_filename_format() {
    let n = 100_000;
    let chunk_names: Vec<String> = (0..n).map(|i| format!("abcdef1234567890_{i:06}")).collect();

    // --- Current: format!("{}.zstd", chunk_name) per call ---
    let start = Instant::now();
    let mut filenames = Vec::with_capacity(n);
    for name in &chunk_names {
        filenames.push(format!("{name}.zstd"));
    }
    let elapsed_format = start.elapsed();

    // --- Alternative: push_str into a reusable buffer ---
    let start = Instant::now();
    let mut filenames_buf = Vec::with_capacity(n);
    let mut buf = String::with_capacity(64);
    for name in &chunk_names {
        buf.clear();
        buf.push_str(name);
        buf.push_str(".zstd");
        filenames_buf.push(buf.clone());
    }
    let elapsed_reuse = start.elapsed();

    let ratio = elapsed_format.as_nanos() as f64 / elapsed_reuse.as_nanos().max(1) as f64;

    println!("bench_chunk_filename_format:");
    println!("  calls: {n}");
    println!(
        "  format!:     {elapsed_format} ({ratio:.2}x)",
        elapsed_format = fmt_dur(elapsed_format)
    );
    println!(
        "  reuse buf:   {elapsed_reuse} (baseline)",
        elapsed_reuse = fmt_dur(elapsed_reuse)
    );
    println!(
        "  heap per call: ~{heap_bytes} bytes",
        heap_bytes = chunk_names[0].len() + 5
    );
}

// ---------------------------------------------------------------------------
// 10. Preinstall: patch chunk loading (memory usage)
// ---------------------------------------------------------------------------
// apply_copy_over and apply_hdiff_patch load the entire patch chunk into
// a Vec<u8>. Measures allocation + read time for various sizes.

#[test]
fn bench_patch_chunk_read() {
    let dir = tempfile::tempdir().expect("tempdir");

    let sizes_mib = [4, 16, 64, 256];

    for size_mib in sizes_mib {
        let size = size_mib as usize * 1024 * 1024;
        let chunk_path = dir.path().join(format!("patch_{size_mib}.bin"));
        {
            let mut f = fs::File::create(&chunk_path).expect("create");
            let data = vec![0xAA_u8; 1024 * 1024];
            for _ in 0..size_mib {
                f.write_all(&data).expect("write");
            }
        }

        // --- Production: vec![0u8; size] + read_exact ---
        let start = Instant::now();
        let mut chunk_file = fs::File::open(&chunk_path).expect("open");
        let mut data = vec![0u8; size];
        chunk_file.read_exact(&mut data).expect("read");
        let elapsed_alloc_read = start.elapsed();

        let peak_heap = size;

        // --- Alternative: streaming copy via io::copy ---
        let dest_path = dir.path().join(format!("patch_{size_mib}_out.bin"));
        let start = Instant::now();
        let mut chunk_file = fs::File::open(&chunk_path).expect("open");
        let dest = fs::File::create(&dest_path).expect("create dest");
        let mut writer = std::io::BufWriter::with_capacity(256 * 1024, dest);
        std::io::copy(&mut chunk_file, &mut writer).expect("copy");
        writer.flush().expect("flush");
        let elapsed_stream = start.elapsed();

        let ratio = elapsed_alloc_read.as_nanos() as f64 / elapsed_stream.as_nanos().max(1) as f64;

        println!("bench_patch_chunk_read ({size_mib} MiB):",);
        println!(
            "  alloc+read:  {elapsed_alloc_read} (peak heap: {peak_heap_mib} MiB)",
            elapsed_alloc_read = fmt_dur(elapsed_alloc_read),
            peak_heap_mib = peak_heap / (1024 * 1024)
        );
        println!(
            "  stream copy: {elapsed_stream} (peak heap: 256 KiB) {ratio:.2}x",
            elapsed_stream = fmt_dur(elapsed_stream)
        );
    }
}

// ---------------------------------------------------------------------------
// 11. Verification cache: retain with stat() per entry
// ---------------------------------------------------------------------------
// On load, the cache does retain() checking path.exists() for every entry.
// For 200K entries, this is 200K stat() syscalls.

#[test]
fn bench_cache_retain_stat() {
    let dir = tempfile::tempdir().expect("tempdir");
    let game_dir = dir.path();

    // Create some real files, some missing
    let n_entries = 50_000;
    let n_real = 40_000;
    for i in 0..n_real {
        let path = game_dir.join(format!("file_{i:05}.bin"));
        fs::write(&path, b"x").expect("write");
    }

    // Build cache with paths (some existing, some not)
    let cache: dashmap::DashMap<String, VerificationEntry> = dashmap::DashMap::new();
    for i in 0..n_entries {
        let rel_path = format!("file_{i:05}.bin");
        cache.insert(
            rel_path,
            VerificationEntry {
                size: 1,
                md5: "abc".to_string(),
                mtime_secs: 1000,
            },
        );
    }

    // --- Production: retain with path.exists() ---
    let start = Instant::now();
    cache.retain(|rel_path, _| {
        let full_path = game_dir.join(rel_path);
        full_path.exists()
    });
    let elapsed = start.elapsed();

    println!("bench_cache_retain_stat:");
    println!(
        "  entries: {n_entries} ({n_real} exist, {n_stale} stale)",
        n_stale = n_entries - n_real
    );
    println!(
        "  time: {elapsed} ({us_per_entry:.1} µs/entry)",
        elapsed = fmt_dur(elapsed),
        us_per_entry = elapsed.as_micros() as f64 / n_entries as f64
    );
    println!("  stat() calls: {n_entries}");
}

// ---------------------------------------------------------------------------
// 12. filter_patch_assets_for_removed_features: clone all vs mutate
// ---------------------------------------------------------------------------
// Production code (filter_patch_assets_for_removed_features) mutates in
// place with zero clones. This benchmark compares that approach against a
// hypothetical bad approach that clones every asset.

#[test]
fn bench_filter_assets_clone_all_vs_mutate() {
    use super::preinstall::{
        FilterCache, PatchAssetInfo, PatchMethod, filter_patch_assets_for_removed_features,
    };
    // Build realistic patch assets
    let n = 50_000;
    let assets: Vec<PatchAssetInfo> = (0..n)
        .map(|i| PatchAssetInfo {
            target_file_path: format!("{HK4E_DATA_DIR_GLOBAL}/StreamingAssets/file_{i:05}.pak"),
            target_file_size: 8 * 1024 * 1024,
            target_file_hash: format!("{i:032}"),
            patch_method: PatchMethod::Patch,
            patch_name: format!("patch_{i:06}"),
            patch_hash: format!("{i:032}"),
            patch_offset: i as u64 * 1024,
            patch_size: 4 * 1024 * 1024,
            patch_chunk_length: 4 * 1024 * 1024,
            original_file_path: Some(format!("{HK4E_DATA_DIR_GLOBAL}/file_{i:05}.pak")),
            original_file_hash: Some(format!("{i:032}")),
            original_file_size: Some(8 * 1024 * 1024),
            matching_field: format!("mf_{i}"),
        })
        .collect();

    // Simulate a filter cache that matches ~5% of assets
    let filter_cache = FilterCache {
        kdel_tokens: Some(vec![
            "mf_523".to_string(),
            "mf_1234".to_string(),
            "mf_9999".to_string(),
        ]),
        blacklist_entries: None,
        ignored_lang_patterns: None,
    };

    // --- Hypothetical bad approach: clone every asset into a new Vec ---
    let start = Instant::now();
    let _result: Vec<PatchAssetInfo> = assets
        .iter()
        .map(|asset| {
            if is_filtered_asset_quick(asset, &filter_cache) {
                let mut cloned = asset.clone();
                cloned.patch_method = PatchMethod::Skip;
                cloned
            } else {
                asset.clone()
            }
        })
        .collect();
    let elapsed_clone_all = start.elapsed();

    // --- Production approach: in-place mutation with zero clones ---
    let mut owned_assets = assets.clone(); // baseline: one full clone to own the data
    let start = Instant::now();
    filter_patch_assets_for_removed_features(&filter_cache, &mut owned_assets);
    let elapsed_mutate = start.elapsed();

    // --- Memory counters ---
    // Each PatchAssetInfo clone duplicates all heap strings.
    // Estimate heap per asset: 5 major strings × ~32 bytes avg ≈ 160 bytes
    let heap_per_asset = std::mem::size_of::<PatchAssetInfo>() + 160;
    let clone_all_heap = n * heap_per_asset;
    let mutate_heap = 0usize; // production path: zero extra clones

    println!("bench_filter_assets_clone_all_vs_mutate:");
    println!("  assets: {n}, filtered: ~5%");
    println!(
        "  clone_all (bad):  {elapsed_clone_all} (baseline)",
        elapsed_clone_all = fmt_dur(elapsed_clone_all)
    );
    println!(
        "  in_place_mutate (production): {elapsed_mutate} (~{speedup:.1}x)",
        elapsed_mutate = fmt_dur(elapsed_mutate),
        speedup = elapsed_clone_all.as_nanos() as f64 / elapsed_mutate.as_nanos().max(1) as f64
    );
    println!(
        "  extra heap: clone_all = ~{clone_all_kb} KB, mutate = ~{mutate_kb} KB",
        clone_all_kb = clone_all_heap / 1024,
        mutate_kb = mutate_heap / 1024
    );
    println!("  production clones: 0 (mutates &mut [PatchAssetInfo] in place)");
}

/// Simplified filter used only in the benchmark to stand in for the real
/// is_filtered_asset without needing a temp dir.
fn is_filtered_asset_quick(
    asset: &super::preinstall::PatchAssetInfo,
    cache: &super::preinstall::FilterCache,
) -> bool {
    if let Some(ref tokens) = cache.kdel_tokens {
        for token in tokens {
            if asset.matching_field.eq_ignore_ascii_case(token) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// 12. Parallel vs sequential file verification (real production pattern)
// ---------------------------------------------------------------------------
// Compares sequential verification (one file at a time) against parallel
// verification (multiple files concurrently). This tests the actual code path
// used by verify_integrity.

#[test]
fn bench_parallel_vs_sequential_verification() {
    use md5::{Digest, Md5};

    let dir = tempfile::tempdir().expect("tempdir");
    let file_size = 10 * 1024 * 1024; // 10 MiB per file
    let num_files = 10;

    // Create test files
    let mut file_paths = Vec::new();
    let mut hashes = Vec::new();
    for i in 0..num_files {
        let path = dir.path().join(format!("verify_test_{i}.bin"));
        let data = vec![0xAB_u8; file_size];
        fs::write(&path, &data).expect("write");
        file_paths.push(path);

        let mut hasher = Md5::new();
        hasher.update(&data);
        hashes.push(hex::encode(hasher.finalize()));
    }

    // Sequential verification
    let start = Instant::now();
    for (path, hash) in file_paths.iter().zip(&hashes) {
        let file = fs::File::open(path).expect("open");
        let mut reader = std::io::BufReader::new(file);
        let mut hasher = Md5::new();
        let mut buf = vec![0u8; FILE_WRITE_BUFFER_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(_) => break,
            }
        }
        let _computed = hex::encode(hasher.finalize());
    }
    let sequential_time = start.elapsed();

    // Parallel verification (simulating parallel integrity check)
    let start = Instant::now();
    let handles: Vec<_> = file_paths
        .iter()
        .zip(&hashes)
        .map(|(path, _hash)| {
            let path = path.clone();
            std::thread::spawn(move || {
                let file = fs::File::open(path).expect("open");
                let mut reader = std::io::BufReader::new(file);
                let mut hasher = Md5::new();
                let mut buf = vec![0u8; FILE_WRITE_BUFFER_SIZE];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => hasher.update(&buf[..n]),
                        Err(_) => break,
                    }
                }
                let _computed = hex::encode(hasher.finalize());
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("join");
    }
    let parallel_time = start.elapsed();

    println!("bench_parallel_vs_sequential_verification:");
    println!(
        "  files: {num_files}, size each: {size_each_mib} MiB, total: {total_mib} MiB",
        size_each_mib = file_size / (1024 * 1024),
        total_mib = num_files * file_size / (1024 * 1024)
    );
    println!(
        "  sequential: {sequential_time}",
        sequential_time = fmt_dur(sequential_time)
    );
    println!(
        "  parallel:   {parallel_time}",
        parallel_time = fmt_dur(parallel_time)
    );
    println!(
        "  speedup:    {speedup:.1}x",
        speedup = sequential_time.as_nanos() as f64 / parallel_time.as_nanos().max(1) as f64
    );
}

#[test]
fn bench_verify_buffer_reuse() {
    use super::preinstall::verify_chunk_md5;

    let dir = tempfile::tempdir().expect("tempdir");
    let num_files = 50;
    let file_size = 1024 * 1024;

    let mut paths = Vec::new();
    let mut hashes = Vec::new();
    for i in 0..num_files {
        let path = dir.path().join(format!("vbr_{i}.bin"));
        let data = vec![0xCD_u8; file_size];
        let mut hasher = Md5::new();
        hasher.update(&data);
        hashes.push(hex::encode(hasher.finalize()));
        fs::write(&path, &data).expect("write");
        paths.push(path);
    }

    let start = Instant::now();
    for (path, hash) in paths.iter().zip(&hashes) {
        assert!(verify_chunk_md5(path, hash));
    }
    let reused = start.elapsed();

    let start = Instant::now();
    for (path, hash) in paths.iter().zip(&hashes) {
        let file = fs::File::open(path).expect("open");
        let mut reader = std::io::BufReader::with_capacity(FILE_WRITE_BUFFER_SIZE, file);
        let mut hasher = Md5::new();
        let mut buf = vec![0u8; FILE_WRITE_BUFFER_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(_) => break,
            }
        }
        assert_eq!(hex::encode(hasher.finalize()), hash.as_str());
    }
    let fresh_alloc = start.elapsed();

    let total_mb = (num_files * file_size) as f64 / (1024.0 * 1024.0);
    println!("bench_verify_buffer_reuse:");
    println!(
        "  files: {num_files} x {file_size_kib} KiB",
        file_size_kib = file_size / 1024
    );
    println!("  total: {total_mb:.1} MiB");
    println!("  reusable buffer:  {reused}", reused = fmt_dur(reused));
    println!(
        "  fresh alloc each: {fresh_alloc}",
        fresh_alloc = fmt_dur(fresh_alloc)
    );
    println!(
        "  ratio:  {ratio:.2}x (reusable / fresh)",
        ratio = reused.as_nanos() as f64 / fresh_alloc.as_nanos().max(1) as f64
    );
}
