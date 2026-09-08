use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use reqwest::Client;
use serde::de::DeserializeOwned;
use tokio::io::AsyncWriteExt;
use url::Url;
use uuid::Uuid;

use crate::core::fs::{
    BaseDirectory::{self, AppData},
    MultiPathOptions, full_path, rename,
};

pub struct DownloadProgress {
    pub download_id: Uuid,
    pub downloaded: u64,
    pub total: u64,
}

/// Downloads and saves a file from a url to a path relative to a base
/// directory. Provides a callback that provides download progress and total
/// file download size
pub async fn download_file(
    url: String,
    dest: PathBuf,
    base_dir: Option<BaseDirectory>,
    on_progress: Option<Box<dyn Fn(DownloadProgress) + Send + 'static>>,
) -> Result<()> {
    ensure!(is_url(&url), "The string {} is not a valid URL!", &url);

    let client = Client::builder().build()?;
    let uuid = Uuid::new_v4();
    let res = client.get(url).send().await?;

    let download_path = PathBuf::from(format!("{uuid}"));
    let status = res.status();
    ensure!(
        status.is_success(),
        "The http request was not successful (Status code {})",
        status
    );

    let size = res.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(full_path(Some(download_path.clone()), None)?).await?;
    let mut downloaded_bytes: u64 = 0;
    let mut stream = res.bytes_stream();

    let mut last_update = Instant::now() - Duration::from_millis(250);
    let throttle = Duration::from_millis(250);

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let c = chunk?;
        file.write_all(&c).await?;
        downloaded_bytes += c.len() as u64;

        // Send the callback data only if one was defined by the function that
        // called it and only after the update timeout is over
        if last_update.elapsed() >= throttle
            && let Some(ref cb) = on_progress
        {
            last_update = Instant::now();
            cb(DownloadProgress {
                download_id: uuid,
                downloaded: downloaded_bytes,
                total: size,
            });
        }
    }

    file.flush().await?;
    rename(MultiPathOptions {
        init_path: download_path,
        init_path_base_dir: Some(AppData),
        dest_path: dest,
        dest_path_base_dir: base_dir,
        overwrite: Some(true),
    })?;
    Ok(())
}

/// Makes a request to an api endpoint and parses it to a desired struct
pub async fn fetch_data<T>(url: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    ensure!(is_url(&url), "The string {} is not a valid URL", url);
    let res = reqwest::get(url).await?;

    ensure!(
        res.status().is_success(),
        "The http request was not successful (Status code {})",
        res.status()
    );

    let data = res.json::<T>().await?;
    Ok(data)
}

/// Checks if a provided string is in the format of a url. It does not check
/// if the destination is a real location
fn is_url(url: &str) -> bool {
    match Url::parse(url) {
        Ok(_) => true,
        Err(_) => false,
    }
}
