use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};
use tokio::net::TcpListener;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

static APP_DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static SERVER_PORT: OnceLock<u16> = OnceLock::new();

fn mime_from_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

struct FileRange {
    data: Vec<u8>,
    start: u64,
    end: u64,
    total: u64,
    mime: &'static str,
    is_partial: bool,
}

fn read_file_range(
    path: PathBuf,
    range_header: Option<String>,
) -> Result<FileRange, StatusCode> {
    let mut file = File::open(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let metadata = file.metadata().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let file_len = metadata.len();
    let mime = mime_from_path(&path);

    let (start, end, is_partial) = if let Some(range) = &range_header {
        if let Some(spec) = range.strip_prefix("bytes=") {
            let parts: Vec<&str> = spec.split('-').collect();
            if parts.len() == 2 {
                let s: u64 = parts[0].parse().unwrap_or(0);
                let e: u64 = if parts[1].is_empty() {
                    file_len - 1
                } else {
                    parts[1].parse().unwrap_or(file_len - 1).min(file_len - 1)
                };
                (s, e, true)
            } else {
                (0, file_len - 1, false)
            }
        } else {
            (0, file_len - 1, false)
        }
    } else {
        (0, file_len - 1, false)
    };

    let content_len = (end - start + 1) as usize;
    file.seek(SeekFrom::Start(start)).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut buf = vec![0u8; content_len];
    file.read_exact(&mut buf).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(FileRange {
        data: buf,
        start,
        end,
        total: file_len,
        mime,
        is_partial,
    })
}

fn build_response(range: FileRange) -> Response<Full<Bytes>> {
    let status = if range.is_partial {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };

    let mut builder = Response::builder()
        .status(status)
        .header("Content-Type", range.mime)
        .header("Content-Length", range.data.len())
        .header("Accept-Ranges", "bytes")
        .header("Cache-Control", "no-cache");

    if range.is_partial {
        builder = builder.header(
            "Content-Range",
            format!("bytes {}-{}/{}", range.start, range.end, range.total),
        );
    }

    builder.body(Full::new(Bytes::from(range.data))).unwrap()
}

fn error_response(status: StatusCode) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::new()))
        .unwrap()
}

async fn handle(
    req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, BoxError> {
    let base = APP_DATA_DIR.get().unwrap();
    let path = req.uri().path();

    let relative = path.strip_prefix('/').unwrap_or(path);
    let file_path = base.join(relative);

    if !file_path.starts_with(base) {
        return Ok(error_response(StatusCode::FORBIDDEN));
    }

    let range_header = req
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    let fp = file_path.clone();
    let result = tokio::task::spawn_blocking(move || read_file_range(fp, range_header))
        .await
        .map_err(|e| -> BoxError { e.into() })?;

    match result {
        Ok(range) => Ok(build_response(range)),
        Err(status) => Ok(error_response(status)),
    }
}

pub async fn start_server(app_handle: AppHandle) -> Result<u16, String> {
    let app_data = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?;

    APP_DATA_DIR
        .set(app_data)
        .map_err(|_| "app data dir already set".to_string())?;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;

    let port = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();

    SERVER_PORT
        .set(port)
        .map_err(|_| "server port already set".to_string())?;

    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let io = TokioIo::new(stream);
            tokio::spawn(async move {
                let _ = http1::Builder::new()
                    .serve_connection(io, service_fn(handle))
                    .await;
            });
        }
    });

    Ok(port)
}

#[tauri::command]
pub fn media_server_port() -> Result<u16, String> {
    SERVER_PORT
        .get()
        .copied()
        .ok_or_else(|| "media server not running".to_string())
}
