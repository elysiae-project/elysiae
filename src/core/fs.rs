use flate2::read::GzDecoder as Gz;
use fs_extra::dir::get_size;
use sha256::try_digest;
use std::{
    fs::{self, create_dir_all},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};
use tar::Archive as Tar;
use xz::read::XzDecoder as Xz;
use zip::ZipArchive as Zip;
use zstd::Decoder as Zstd;

use anyhow::{Context, Result, bail, ensure};
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
/// Desktop: configured user desktop directory
///
/// Home: ~
///
/// Compat: ~/.local/share/elysiae/proton-data
#[derive(PartialEq, Debug, Clone, Copy)]
pub enum BaseDirectory {
    AppData,
    Desktop,
    Home,
    Compat,
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum Sizes {
    Bytes,
    Kilobytes,
    Megabytes,
    Gigabytes,
    Terabytes,
}

impl Sizes {
    fn power(self) -> i32 {
        self as i32
    }
}

#[cfg(not(test))]
static BASE_DIRS: OnceLock<BaseDirs> = OnceLock::new();

#[cfg(not(test))]
fn base_dirs() -> Result<BaseDirs> {
    if let Some(dirs) = BASE_DIRS.get() {
        return Ok(dirs.clone());
    }
    let dirs = BaseDirs::new().context("home directory unavailable")?;
    let _ = BASE_DIRS.set(dirs.clone());
    Ok(dirs)
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
    let parent_dir = fp
        .parent()
        .context("The target path has no parent directory")?;
    if !parent_dir.try_exists()? {
        create_dir_all(parent_dir).context("Failed to create parent directories")?;
    }

    let temp_path = parent_dir.join(format!(
        ".{}-tmp-{}",
        fp.file_name()
            .context("The target path has no filename")?
            .to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    fs::write(&temp_path, contents).context("Failed to write temporary file")?;
    fs::rename(temp_path, fp).context("Failed to atomically replace the path")?;

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
        ensure!(fp.is_dir(), "The existing path is not a directory");
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

    if dfp.try_exists()? {
        ensure!(
            (ifp.is_dir() && dfp.is_dir()) || (ifp.is_file() && dfp.is_file()),
            "{} and {} are two different item types",
            ifp.to_string_lossy(),
            dfp.to_string_lossy()
        );
    } else if let Some(parent) = dfp.parent() {
        create_dir_all(parent).context("Failed to create destination parent")?;
    }

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
    if !fp.try_exists()? {
        return Ok(());
    }
    if fp.is_file() {
        fs::remove_file(fp).context("Could not remove this file")?;
    } else if fp.is_dir() {
        match recursive {
            Some(true) | None => {
                fs::remove_dir_all(fp).context("Could not recursively delete this directory")?
            }
            Some(false) => fs::remove_dir(fp).context("Could not remove this directory")?,
        };
    }
    Ok(())
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
pub async fn extract_file(options: MultiPathOptions, flatten: Option<bool>) -> Result<()> {
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

    let ifp_ext = ifp
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .context("The archive has no valid UTF-8 extension")?;

    tokio::task::spawn_blocking(move || -> Result<()> {
        let file = fs::File::open(&ifp).context("Could not open the initial path")?;
        if !dfp.exists() {
            fs::create_dir_all(&dfp).context("Could not create the extraction directory")?;
        }

        // Archive formats handled by Elysiae are tarballs and zip files. Fail
        // explicitly for unknown extensions instead of silently reporting success.
        match ifp_ext.as_str() {
            "gz" | "tgz" => Tar::new(Gz::new(file)).unpack(&dfp)?,
            "xz" | "txz" => Tar::new(Xz::new(file)).unpack(&dfp)?,
            "zst" | "zstd" => Tar::new(Zstd::new(file)?).unpack(&dfp)?,
            "zip" => Zip::new(file)?.extract(&dfp)?,
            extension => bail!("Unsupported archive extension: .{extension}"),
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
    })
    .await
    .context("Extraction task panicked or was cancelled")?
}

/// Validates the integrity of a file relative to a base bath against the
/// expected sha256sum of the file. If no base directory is provided, the base
/// directory will default to the App Data Directory (~/.local/share/elysiae)
pub async fn verify_sha256sum(
    file: PathBuf,
    base_dir: Option<BaseDirectory>,
    expected_sum: String,
) -> Result<bool> {
    let fp = full_path(Some(file), base_dir).context("Could not resolve full path")?;

    // Get the file hash. If getting the hash from the file fails,
    // default to an empty string, which can indicate to the function
    // that the file hashes do not match
    tokio::task::spawn_blocking(move || -> Result<bool> {
        let fh = try_digest(fp).unwrap_or("".to_string());

        Ok(fh.eq(&expected_sum))
    })
    .await
    .context("Failed to verify integrity of file")?
}

/// Gets the size of a directory, relative to a Base directory. Size is returned
/// in a specified unit (default: Bytes)
///
/// If no base directory is provided, the function will default to the app data
/// directory (~/.local/share/elysiae)
pub async fn get_dir_size(
    p: PathBuf,
    base_dir: Option<BaseDirectory>,
    target_unit: Option<Sizes>,
) -> Result<f64> {
    let fp = full_path(Some(p), base_dir).context("The path could not be resolved")?;
    tokio::task::spawn_blocking(move || -> Result<f64> {
        let raw_size = get_size(fp).context("Could not get size of directory")? as f64;

        Ok(size_as(
            raw_size,
            Sizes::Bytes,
            target_unit.unwrap_or(Sizes::Bytes),
        ))
    })
    .await
    .context("Failed to get size of directory")?
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
    let dir_path = base_path(base_dir)?;
    match p {
        Some(relative) => join_beneath(dir_path, relative),
        None => Ok(dir_path),
    }
}

#[cfg(not(test))]
fn base_path(base_dir: Option<BaseDirectory>) -> Result<PathBuf> {
    let d = base_dirs()?;
    Ok(match base_dir {
        Some(x) => match x {
            BaseDirectory::AppData => d.data_local_dir().to_path_buf().join("elysiae"),
            BaseDirectory::Desktop => directories::UserDirs::new()
                .context("user directories unavailable")?
                .desktop_dir()
                .context("desktop directory is not configured")?
                .to_path_buf(),
            BaseDirectory::Home => d.home_dir().to_path_buf(),
            BaseDirectory::Compat => d
                .data_local_dir()
                .to_path_buf()
                .join("elysiae")
                .join("proton-data"),
        },
        None => d.data_local_dir().to_path_buf().join("elysiae"),
    })
}

// Test builds never resolve default paths into a real user profile, including
// when a test forgets to configure its own fixture. Do not mutate HOME in tests:
// Tokio workers and parallel tests share the process environment.
#[cfg(test)]
fn base_path(base_dir: Option<BaseDirectory>) -> Result<PathBuf> {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    let home = ROOT.get_or_init(|| {
        std::env::temp_dir().join(format!("elysiae-tests-{}", uuid::Uuid::new_v4()))
    });
    Ok(match base_dir.unwrap_or(BaseDirectory::AppData) {
        BaseDirectory::Home => home.clone(),
        BaseDirectory::Desktop => home.join("Desktop"),
        BaseDirectory::AppData => home.join(".local/share/elysiae"),
        BaseDirectory::Compat => home.join(".local/share/elysiae/proton-data"),
    })
}

/// Converts the size of something in the filesystem between units, primarilly
/// intended for use in the frontend where displaying everything as bytes isn't
/// as fashionable as it is in the backend
///
/// This function can convert between all values between bytes and terabytes.
/// Support for larger units isn't needed because no games are even remotely close to the
/// petabyte and beyond range as of writing
pub fn size_as(initial_size: f64, initial_unit: Sizes, new_unit: Sizes) -> f64 {
    (initial_size * (1024.0 as f64).powi(initial_unit.power() - new_unit.power()))
}

/// Joins an untrusted relative path to a trusted (possibly XDG-configured) base.
fn join_beneath(base: PathBuf, relative: PathBuf) -> Result<PathBuf> {
    ensure!(
        !relative.as_os_str().is_empty()
            && !relative.is_absolute()
            && relative
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "path must be a nonempty relative path without '.' or '..': {}",
        relative.display()
    );

    // Resolve the base independently: XDG directories and their ancestors may
    // legitimately be symlinks. Only paths *within* this base are untrusted.
    let root = canonicalize_with_missing(&base)?;
    let mut current = root.clone();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(_) => {
                current = current.canonicalize()?;
                ensure!(
                    current.starts_with(&root),
                    "path escapes its base directory"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(base.join(relative))
}

fn canonicalize_with_missing(path: &Path) -> Result<PathBuf> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let mut resolved = ancestor.canonicalize()?;
                for name in missing.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .context("path has no existing ancestor")?,
                );
                ancestor = ancestor.parent().context("path has no existing ancestor")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    fn generate_random_data() -> Vec<u8> {
        let mut file_data = vec![0u8; rand::random_range(100..1000) as usize];
        rand::rng().fill_bytes(&mut file_data);
        file_data
    }

    #[test]
    fn test_full_path() {
        let home = base_path(Some(BaseDirectory::Home)).unwrap();
        assert_ne!(home, BaseDirs::new().unwrap().home_dir());
        assert!(home.starts_with(std::env::temp_dir()));
        let dummy_dir = "dummy-dir";

        // Home
        let expected_home = home.join(dummy_dir);
        let test_home_dir = full_path(Some(dummy_dir.into()), Some(BaseDirectory::Home)).unwrap();
        assert_eq!(test_home_dir, expected_home);

        // Desktop
        let expected_desktop = home.join("Desktop").join(dummy_dir);
        let test_desktop_dir =
            full_path(Some(dummy_dir.into()), Some(BaseDirectory::Desktop)).unwrap();
        assert_eq!(test_desktop_dir, expected_desktop);

        // App Data
        let expected_app_data = home.join(".local/share/elysiae").join(dummy_dir);
        let test_app_data_dir =
            full_path(Some(dummy_dir.into()), Some(BaseDirectory::AppData)).unwrap();
        assert_eq!(test_app_data_dir, expected_app_data);

        // Compat
        let expected_compat = home
            .join(".local/share/elysiae/proton-data")
            .join(dummy_dir);
        let test_compat_dir =
            full_path(Some(dummy_dir.into()), Some(BaseDirectory::Compat)).unwrap();
        assert_eq!(test_compat_dir, expected_compat);
    }

    #[test]
    fn file_write_exists_read_delete() {
        // Write a new file
        let path: PathBuf = uuid::Uuid::new_v4().to_string().into();
        let mut contents: Vec<u8> = generate_random_data();
        write_file(path.clone(), &contents, Some(BaseDirectory::Home)).unwrap();

        // Check if the file exists
        assert_eq!(
            exists(path.clone(), Some(BaseDirectory::Home)).unwrap(),
            true
        );

        // Read file
        let pass_one_contents = read_file(path.clone(), Some(BaseDirectory::Home)).unwrap();
        assert_eq!(pass_one_contents, contents.to_owned());

        // Overwrite file with new data
        contents = generate_random_data();
        write_file(path.clone(), &contents, Some(BaseDirectory::Home)).unwrap();

        // Read file again
        let pass_two_contents = read_file(path.clone(), Some(BaseDirectory::Home)).unwrap();
        assert_ne!(pass_one_contents, pass_two_contents);
        assert_eq!(pass_two_contents, contents);

        // Delete file
        remove(path.clone(), Some(BaseDirectory::Home), None).unwrap(); //
        assert_eq!(exists(path, Some(BaseDirectory::Home)).unwrap(), false)
    }

    #[test]
    fn dir_write_exists_read_delete() {
        let directory = PathBuf::from(format!("elysiae-test-{}", uuid::Uuid::new_v4()));
        let file = directory.join("nested.txt");
        mkdir(directory.clone(), Some(BaseDirectory::Home)).unwrap();
        mkdir(directory.clone(), Some(BaseDirectory::Home)).unwrap();
        write_file(file.clone(), b"directory test", Some(BaseDirectory::Home)).unwrap();
        assert!(
            read_dir(directory.clone(), Some(BaseDirectory::Home))
                .unwrap()
                .contains(&full_path(Some(file), Some(BaseDirectory::Home)).unwrap())
        );
        remove(directory, Some(BaseDirectory::Home), Some(true)).unwrap();
    }

    #[test]
    fn path_validation_rejects_escape_attempts() {
        assert!(full_path(Some(PathBuf::from("../outside")), None).is_err());
        assert!(full_path(Some(PathBuf::from("/absolute")), None).is_err());
        assert!(full_path(Some(PathBuf::from(".")), None).is_err());
        assert!(full_path(Some(PathBuf::new()), None).is_err());
    }

    #[test]
    fn empty_remove_cannot_delete_base() {
        let sentinel = PathBuf::from(format!("sentinel-{}", uuid::Uuid::new_v4()));
        write_file(sentinel.clone(), b"keep", None).unwrap();
        assert!(remove(PathBuf::new(), None, Some(true)).is_err());
        assert_eq!(read_file(sentinel.clone(), None).unwrap(), b"keep");
        remove(sentinel, None, None).unwrap();
    }

    #[test]
    fn path_validation_rejects_escaping_symlink_ancestors() {
        let root = base_path(None)
            .unwrap()
            .join(uuid::Uuid::new_v4().to_string());
        let owned = root.join("owned");
        let outside = root.join("outside");
        fs::create_dir_all(owned.join("internal")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, owned.join("escape")).unwrap();
        std::os::unix::fs::symlink("internal", owned.join("safe")).unwrap();
        std::os::unix::fs::symlink(outside.join("missing"), owned.join("dangling")).unwrap();
        for path in [
            "escape",
            "escape/sentinel",
            "escape/new/file",
            "dangling/new",
        ] {
            assert!(join_beneath(owned.clone(), path.into()).is_err(), "{path}");
        }
        assert!(join_beneath(owned.clone(), "safe/new/file".into()).is_ok());
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_validation_allows_configured_symlink_bases_and_future_directories() {
        let root = base_path(None)
            .unwrap()
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(root.join("data")).unwrap();
        std::os::unix::fs::symlink(root.join("data"), root.join("xdg")).unwrap();
        for base in [root.join("xdg"), root.join("xdg/elysiae/future")] {
            assert_eq!(
                join_beneath(base.clone(), "game/file".into()).unwrap(),
                base.join("game/file")
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn size_conversion_is_reversible() {
        let value = 3.5;
        let bytes = size_as(value, Sizes::Gigabytes, Sizes::Bytes);
        assert!((size_as(bytes, Sizes::Bytes, Sizes::Gigabytes) - value).abs() < f64::EPSILON);
    }
}
