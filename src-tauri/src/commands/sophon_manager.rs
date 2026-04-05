use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::FileExt;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::util::sophon_assets::{self, SophonChunkData};
use crate::util::sophon_assets::{FrontDoorResponse, ManifestResposne, SophonChunk};
use bytes::Bytes;
use protobuf::CodedInputStream;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State, command};
use zstd::Decoder as Zstd;

pub struct HttpClient(pub reqwest::Client);

type ProtoObject = HashMap<String, ProtoValue>;
#[allow(unused)]
#[derive(Debug, Clone)]
enum ProtoValue {
    Uint64(u64),
    Fixed32(u32),
    String(String),
    Message(ProtoObject),
    Array(Vec<ProtoValue>),
}

pub struct FileStore {
    files: RwLock<HashMap<u32, File>>,
    next_id: AtomicU32,
}

impl FileStore {
    #[allow(unused)]
    pub fn new() -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
            next_id: AtomicU32::new(0),
        }
    }
}

#[command]
pub fn open_file(
    path: String,
    total_size: u64,
    store: State<FileStore>,
    app_handle: AppHandle,
) -> Result<u32, String> {
    let full_path = app_handle
        .path()
        .resolve(&path, BaseDirectory::AppData)
        .unwrap();

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&full_path)
        .map_err(|e| e.to_string())?;

    file.set_len(total_size).map_err(|e| e.to_string())?;

    let id = store.next_id.fetch_add(1, Ordering::Relaxed);
    store.files.write().unwrap().insert(id, file);
    Ok(id)
}

#[command(async)]
pub async fn download_and_write_chunk(
    file_id: u32,
    cdn_url: String,
    offset: u64,
    store: State<'_, FileStore>,
    client: State<'_, HttpClient>,
) -> Result<(), String> {
    let compressed = client
        .0
        .get(&cdn_url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;

    // Clone the file handle and let the store reference drop before spawn_blocking
    let file = {
        let files = store.files.read().unwrap();
        files
            .get(&file_id)
            .ok_or("invalid file id")?
            .try_clone()
            .map_err(|e| e.to_string())?
    };

    tokio::task::spawn_blocking(move || {
        let decompressed = decompress_zstd_item(&compressed).map_err(|e| e.to_string())?;

        file.write_at(&decompressed, offset)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(())
}

#[command]
pub fn close_file(id: u32, store: State<FileStore>) {
    store.files.write().unwrap().remove(&id);
}

#[command(async)]
pub async fn get_all_chunks(game_id: String, vo_lang: String) -> Result<Vec<SophonChunk>, String> {
    // Basically just decode the protobuf downloaded for the requested game + its voiceovers (in the requested language)
    // Tauri serialises this on the frontend as an object, so I can just have a function that returns SophonChunk[] or something

    // The rest of the file outside of this function and download_parse_manifest are just helper functions that format a protobuf file to be
    // able to get returned as the struct. Too much for my brain to understand

    let manifest_json = manifest_endpoint_data(&game_id)
        .await
        .map_err(|e| e.to_string())?;

    let mut manifest = download_parse_manifest(&manifest_json.data.manifests[0])
        .await
        .map_err(|e| e.to_string())?;

    manifest.append(
        &mut download_parse_manifest(
            &manifest_json.data.manifests[vo_manifest_index(&game_id, &vo_lang) as usize],
        )
        .await
        .map_err(|e| e.to_string())?,
    );

    Ok(manifest)
}

async fn download_parse_manifest(
    manifest_data: &sophon_assets::Manifest,
) -> Result<Vec<SophonChunk>, Box<dyn std::error::Error>> {
    let manifest_prefix = &manifest_data.manifest_download.url_prefix;
    let chunk_prefix = &manifest_data.chunk_download.url_prefix;

    let manifest_url = format!("{}/{}", manifest_prefix, manifest_data.manifest.id);
    let compressed_manifest = reqwest::get(manifest_url).await?.bytes().await?;
    let decompressed = decompress_zstd_item(&compressed_manifest)?;

    let proto = decode_protobuf(&decompressed)?;
    Ok(proto_to_sophon_chunks(&proto, chunk_prefix))
}

fn decompress_zstd_item(compressed: &Bytes) -> Result<Vec<u8>, std::io::Error> {
    let mut decoder = Zstd::new(compressed.as_ref())?;
    let mut res = Vec::new();
    decoder.read_to_end(&mut res)?;
    Ok(res)
}

async fn manifest_endpoint_data(
    game_id: &String,
) -> Result<ManifestResposne, Box<dyn std::error::Error>> {
    let front_door_url = format!(
        "https://{}-{}-{}.{}.com/{}/{}-connect/api/getGameBranches?&launcher_id={}",
        "sg", "hyp", "api", "hoyoverse", "hyp", "hyp", "VYTpXlbWo8"
    );

    let front_door_json: FrontDoorResponse =
        serde_json::from_str(&reqwest::get(front_door_url).await?.text().await?)?;

    let game_data = &front_door_json.data.game_branches[front_door_game_index(&game_id) as usize];

    let manifest_endpoint_url = format!(
        "https://sg-public-api.{}.com/downloader/sophon_chunk/api/getBuild?branch=main&package_id={}&password={}",
        "hoyoverse", &game_data.main.package_id, &game_data.main.password
    );

    let manifest_json: ManifestResposne =
        serde_json::from_str(&reqwest::get(manifest_endpoint_url).await?.text().await?)?;

    Ok(manifest_json)
}

fn decode_protobuf(buf: &[u8]) -> Result<ProtoObject, Box<dyn std::error::Error>> {
    let mut reader = CodedInputStream::from_bytes(buf);
    let mut obj: ProtoObject = HashMap::new();

    while !reader.eof()? {
        let tag = reader.read_raw_varint32()?;
        let field = (tag >> 3).to_string();
        let wire = tag & 0x7;

        let value = match wire {
            0 => ProtoValue::Uint64(reader.read_raw_varint64()?),
            1 => ProtoValue::Uint64(reader.read_raw_little_endian64()?),
            2 => {
                let len = reader.read_raw_varint32()?;
                let bytes = reader.read_raw_bytes(len)?;
                match decode_protobuf(&bytes) {
                    Ok(nested) => ProtoValue::Message(nested),
                    Err(_) => ProtoValue::String(String::from_utf8_lossy(&bytes).into_owned()),
                }
            }
            5 => ProtoValue::Fixed32(reader.read_raw_little_endian32()?),
            wt => return Err(format!("Unknown wire type: {wt}").into()),
        };

        match obj.remove(&field) {
            None => {
                obj.insert(field, value);
            }
            Some(ProtoValue::Array(mut arr)) => {
                arr.push(value);
                obj.insert(field, ProtoValue::Array(arr));
            }
            Some(existing) => {
                obj.insert(field, ProtoValue::Array(vec![existing, value]));
            }
        }
    }

    Ok(obj)
}

fn front_door_game_index(id: &String) -> i32 {
    let parsed_id: String = id.to_lowercase();
    match parsed_id.as_str() {
        // TODO: Figure out which indices on the front door endpoint correspond to which bh3 server region
        "bh3" => 3,
        "hk4e" => 2,
        "hkrpg" => 1,
        "nap" => 0,
        _ => -1,
    }
}

fn vo_manifest_index(game: &String, vo_lang: &String) -> i32 {
    if game.contains("bh3") {
        // Since each bh3 region only contains 1 VO language, index 1 of the sophon manifest endpoint will house those assets (index 0 is always the game data)
        return 1;
    }

    match vo_lang.to_lowercase().as_str() {
        "cn" => 1,
        "en" => 2,
        "jp" => 3,
        "kr" => 4,
        _ => -1,
    }
}

fn proto_to_sophon_chunks(obj: &ProtoObject, cdn_prefix: &str) -> Vec<SophonChunk> {
    let raw_files = match obj.get("1") {
        Some(ProtoValue::Message(m)) => vec![m],
        Some(ProtoValue::Array(arr)) => arr
            .iter()
            .filter_map(|v| match v {
                ProtoValue::Message(m) => Some(m),
                _ => None,
            })
            .collect(),
        _ => return vec![],
    };

    raw_files
        .into_iter()
        .map(|file| {
            let chunks = match file.get("2") {
                Some(ProtoValue::Message(m)) => vec![map_chunk_data(m, cdn_prefix)],
                Some(ProtoValue::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| match v {
                        ProtoValue::Message(m) => Some(map_chunk_data(m, cdn_prefix)),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            };

            SophonChunk {
                filename: get_str(file, "1").to_string(),
                size: get_u64(file, "4") as i32,
                md5: get_str(file, "5").to_string(),
                chunks,
            }
        })
        .collect()
}

fn map_chunk_data(c: &ProtoObject, cdn_prefix: &str) -> SophonChunkData {
    SophonChunkData {
        cdn_url: format!("{}/{}", cdn_prefix, get_str(c, "1")),
        compressed_md5: get_str(c, "2").to_string(),
        offset: get_u64(c, "3") as i32,
        compressed_size: get_u64(c, "4") as i32,
        uncompressed_size: get_u64(c, "5") as i32,
        xxhash64: get_u64(c, "6").to_string(),
        uncompressed_md5: get_str(c, "7").to_string(),
    }
}

fn get_str<'a>(obj: &'a ProtoObject, field: &str) -> &'a str {
    match obj.get(field) {
        Some(ProtoValue::String(s)) => s.as_str(),
        _ => "",
    }
}

fn get_u64(obj: &ProtoObject, field: &str) -> u64 {
    match obj.get(field) {
        Some(ProtoValue::Uint64(n)) => *n,
        _ => 0,
    }
}
