use flate2::read::GzDecoder as Gz;
use fs_extra::dir::get_size;
use sha256::try_digest;
use std::{
    fs::{self, DirEntry, create_dir_all},
    path::{Component, PathBuf},
};
use tar::Archive as Tar;
use xz::read::XzDecoder as Xz;
use zip::ZipArchive as Zip;
use zstd::Decoder as Zstd;

use anyhow::{Context, Error, Ok, Result, ensure};
use directories::BaseDirs;
use log::warn;

/// Directories that elysiae commonly uses, provided to make filesystem
/// operations a bit cleaner by only requiring paths relative to these base
/// directories
///
/// Paths:
///
/// AppData: ~/.local/share/elysiae
///
/// Desktop: ~/Desktop
///
/// Home: ~
///
/// Compat: ~/.local/share/elysiae/proton-data
pub enum BaseDirectory {
    AppData,
    Desktop,
    Home,
    Compat,
}

/// Used for getting paths and base directories for filesystem operations that
/// require two paths instead of one
pub struct MultiPathOptions {
    pub init_path: PathBuf,
    pub init_path_base_dir: Option<BaseDirectory>,
    pub dest_path: PathBuf,
    pub dest_path_base_dir: Option<BaseDirectory>,
    pub overwrite: Option<bool>,
}

/// Wrapper function for std::fs::exists(), but relative to a base directory
///
/// If no base directory is provided, the function defaults to the app data
/// directory (~/.local/share/elysiae)
pub fn exists(p: PathBuf, base_dir: Option<BaseDirectory>) -> Result<bool> {
    let fp = full_path(Some(p), base_dir).context("Full path could not be resolved")?;
    Ok(fp.try_exists()?)
}

/// Wrapper function for std::fs::read, relative to a base directory
///
/// If no base directory is provided, the function defaults to the app data
/// directory (~/.local/share/elysiae)
pub fn read_file(p: PathBuf, base_dir: Option<BaseDirectory>) -> Result<Vec<u8>> {
    let fp = full_path(Some(p), base_dir)?;
    ensure!(
        fp.try_exists()?,
        "The Path \"{}\" could not be found on disk",
        fp.to_string_lossy()
    );

    let data = fs::read(fp).context("Could not read this file")?;
    Ok(data)
}

/// Wraper function for std::fs::write, but relative to a base
/// directory
///
/// If no base directory is provided, the function defaults to the app data
/// directory (~/.local/share/elysiae)
pub fn write_file(p: PathBuf, contents: &[u8], base_dir: Option<BaseDirectory>) -> Result<()> {
    let fp = full_path(Some(p), base_dir)?;

    ensure!(
        !fp.is_dir(),
        "The Path \"{}\" is a directory",
        fp.to_string_lossy()
    );

    // Create all directories on the path that don't exist
    let parent_dir = fp.parent().unwrap();
    if !parent_dir.try_exists()? {
        let _ = create_dir_all(parent_dir);
    }

    // Write the contents of the file to the final location
    let _ = fs::write(fp, contents).context("Failed to write to the path")?;

    Ok(())
}

/// Wrapper function for std::fs::create_dir_all, relative to a base directory
///
/// If the path this function is trying to create exists, the function will emit
/// a warning but still completes successfully, as it didn't run into any real
/// errors
///
/// If no base directory is provided, the function defaults to the app data
/// directory (~/.local/share/elysiae)
pub fn mkdir(p: PathBuf, base_dir: Option<BaseDirectory>) -> Result<()> {
    let fp = full_path(Some(p), base_dir).context("File path could not be resolved")?;

    if fp.try_exists()? {
        warn!(
            "The path \"{}\" already exists. No action taken",
            fp.to_string_lossy()
        );
        Ok(())
    } else {
        fs::create_dir_all(fp).context("The path could not be created")?;
        Ok(())
    }
}

/// Wrapper function for std::fs::rename(), relative to a base directory
///
/// Both paths must be of the same type, if they are different an error will be
/// thrown
///
/// If no base directory is provided, the function defaults to the app data
/// directory (~/.local/share/elysiae)
pub fn rename(options: MultiPathOptions) -> Result<()> {
    let ifp = full_path(Some(options.init_path), options.init_path_base_dir)
        .context("Could not resolve initial path")?;
    let dfp = full_path(Some(options.dest_path), options.dest_path_base_dir)
        .context("Could not resolve destination path")?;

    ensure!(
        ifp.try_exists()?,
        "The initial path \"{}\" does not exist",
        ifp.to_string_lossy()
    );

    let allow_overwrites = options.overwrite.unwrap_or(true);

    ensure!(
        !dfp.try_exists()? || allow_overwrites,
        "The destination path \"{}\" already exists and overwriting files has been disabled!",
        dfp.to_string_lossy()
    );

    ensure!(
        (ifp.is_dir() && dfp.is_dir()) || (ifp.is_file() && dfp.is_file()),
        "{} and {} are two different item types",
        ifp.to_string_lossy(),
        dfp.to_string_lossy()
    );

    fs::rename(ifp, dfp)?;
    Ok(())
}

/// Wrapper function for std::fs::remove_file, std::fs::remove_dir and
/// std::fs::remove_dir_all, relative to a base directory
///
/// The function automatically determines weather the path you specified is a
/// directory or not and calls the appropriate std function to remove the item
/// from the filesystem
///
/// If a recursive parameter is not provided, it will automatically recursively
/// delete a directory
///
/// If no base directory is provided, the function will default to the app data
/// directory (~/.local/share/elysiae)
pub fn remove(p: PathBuf, base_dir: Option<BaseDirectory>, recursive: Option<bool>) -> Result<()> {
    let fp = full_path(Some(p), base_dir)?;
    if fp.try_exists()? {
        if fp.is_file() {
            fs::remove_file(fp).context("Could not remove this file")?;
        } else if fp.is_dir() {
            match recursive {
                Some(x) => {
                    if x {
                        fs::remove_dir_all(fp)
                            .context("Could not recursively delete this directory")?;
                    } else {
                        fs::remove_dir(fp).context("Could not remove this directory")?;
                    }
                }
                None => {
                    fs::remove_dir_all(fp)
                        .context("Could not recursively delete this directory")?;
                }
            };
        }
        Ok(())
    } else {
        Err(Error::msg(format!(
            "The Path \"{}\" could not be found on disk",
            fp.to_string_lossy()
        )))
    }
}

/// Extracts a .tar.gz, .tar.xz, .tar.zstd, or .zip file to a specified
/// directory.
///
/// Can automatically "flatten" an extracted file (move all items in a nested
/// directory after extraction one level up), and does so by default. to disable
/// this behaviour, set the flatten parameter to Some(false)
///
/// Both the archive path and destination folder are paths relative to a base
/// directory. Paths that don't provide a base directory parameter default to
/// the app data directory (~/.local/share/elysiae)
pub fn extract_file(options: MultiPathOptions, flatten: Option<bool>) -> Result<()> {
    let ifp = full_path(Some(options.init_path), options.init_path_base_dir)
        .context("Could not resolve initial path")?;
    let dfp = full_path(Some(options.dest_path), options.dest_path_base_dir)
        .context("Could not resolve destination path")?;

    let should_flatten = flatten.unwrap_or(true);

    ensure!(
        ifp.try_exists()?,
        "The initial path \"{}\" does not exist",
        ifp.to_string_lossy()
    );

    let allow_overwrites = options.overwrite.unwrap_or(true);

    ensure!(
        !dfp.try_exists()? || allow_overwrites,
        "The destination path \"{}\" already exists and overwriting files has been disabled!",
        dfp.to_string_lossy()
    );

    let ifp_ext = ifp.extension().unwrap().to_str().unwrap().to_owned();
    let file = fs::File::open(ifp).context("Could not open the initial path")?;

    // Check for comman alternate file extensions of archive files handled by
    // Elysiae Elysiae only deals with tarball archives, so there's no need to
    // check if they are standalone archived files
    if ifp_ext.eq("gz") || ifp_ext.eq("tgz") {
        // Borrow dfp to allow the extracted dir flattening to properly execute later
        Tar::new(Gz::new(file)).unpack(&dfp)?
    } else if ifp_ext.eq("xz") || ifp_ext.eq("txz") {
        Tar::new(Xz::new(file)).unpack(&dfp)?;
    } else if ifp_ext.eq("zst") || ifp_ext.eq("zstd") {
        Tar::new(Zstd::new(file)?).unpack(&dfp)?;
    } else if ifp_ext.eq("zip") {
        Zip::new(file)?.extract(&dfp)?;
    }

    if should_flatten {
        let entries: Vec<_> = std::fs::read_dir(&dfp)?.collect::<Result<_, _>>()?;

        if entries.len() == 1 && entries[0].path().is_dir() {
            let inner_dir = entries[0].path();

            for archive_entry in fs::read_dir(&inner_dir)? {
                let entry = archive_entry?;

                let target = dfp.join(entry.file_name());
                fs::rename(entry.path(), target)?;
            }

            fs::remove_dir(inner_dir)?;
        }
    }

    Ok(())
}

/// Validates the integrity of a file relative to a base bath against the
/// expected sha256sum of the file. If no base directory is provided, the base
/// directory will default to the App Data Directory (~/.local/share/elysiae)
pub fn verify_sha256sum(
    file: PathBuf,
    base_dir: Option<BaseDirectory>,
    expected_sum: String,
) -> Result<bool> {
    let fp = full_path(Some(file), base_dir).context("Could not resolve full path")?;

    // Get the file hash. If getting the hash from the file fails,
    // default to an empty string, which can indicate to the function
    // that the file hashes do not match
    let fh = try_digest(fp).unwrap_or("".to_string());

    Ok(fh.eq(&expected_sum))
}

/// Gets the size of a directory, relative to a Base directory. Size is returned
/// in bytes
///
/// If no base directory is provided, the function will default to the app data
/// directory (~/.local/share/elysiae)
pub fn get_dir_size(p: PathBuf, base_dir: Option<BaseDirectory>) -> Result<u64> {
    let fp = full_path(Some(p), base_dir).context("The path could not be resolved")?;
    let size = get_size(fp).context("Could not get size of directory")?;

    Ok(size)
}

/// Wrapper function for std::fs::read_dir, relative to a user-specified "base directory"
/// Values are returned as a PathBuf Vector rather than a DirEntry Vector because it is more useful to Elysiae in that form
///
/// If no base directory is provided, the function will default to the app data
/// directory (~/.local/share/elysiae)
pub fn read_dir(p: PathBuf, base_dir: Option<BaseDirectory>) -> Result<Vec<PathBuf>> {
    let fp = full_path(Some(p), base_dir).context("The path could not be resolved")?;

    Ok(std::fs::read_dir(fp)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect::<Vec<_>>())
}

/// Joins a specified base directory with a relative path
///
/// If no relative path is provided, the base directory is returned by
/// itself.
///
/// If no base directory is provided, the function will default to the app data
/// directory (~/.local/share/elysiae)
pub fn full_path(p: Option<PathBuf>, base_dir: Option<BaseDirectory>) -> Result<PathBuf> {
    let d = BaseDirs::new().context("Could not find base directories!")?;

    let dir_path = match base_dir {
        Some(x) => match x {
            BaseDirectory::AppData => d.data_local_dir().to_path_buf().join("elysiae"),
            BaseDirectory::Desktop => d.home_dir().join("Desktop"),
            BaseDirectory::Home => d.home_dir().to_path_buf(),
            BaseDirectory::Compat => d
                .data_local_dir()
                .to_path_buf()
                .join("elysiae")
                .join("proton-data"),
        },
        None => d.data_local_dir().to_path_buf().join("elysiae"),
    };

    match p {
        Some(x) => join_beneath(dir_path, x),
        None => Ok(dir_path),
    }
}

fn join_beneath(base: PathBuf, relative: PathBuf) -> Result<PathBuf> {
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "path must be a relative path without '.' or '..': {}",
        relative.display()
    );

    Ok(base.join(relative))
}
