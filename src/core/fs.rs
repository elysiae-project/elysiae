use flate2::read::GzDecoder as Gz;
use fs_extra::dir::get_size;
use sha256::try_digest;
use std::{
    fs::{self, create_dir_all},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};
use tar::Archive as Tar;
use xz::read::XzDecoder as Xz;
use zip::ZipArchive as Zip;
use zstd::Decoder as Zstd;

use anyhow::{Context, Result, bail, ensure};
#[cfg(not(test))]
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

/// Size units used for filesystem measurements and conversions.
#[derive(PartialEq, Debug, Clone, Copy)]
pub enum Sizes {
    Bytes,
    Kilobytes,
    Megabytes,
    Gigabytes,
    Terabytes,
}

impl Sizes {
    /// Returns the unit's exponent for base-1024 conversions.
    fn power(self) -> i32 {
        self as i32
    }
}

#[cfg(not(test))]
static BASE_DIRS: OnceLock<BaseDirs> = OnceLock::new();

/// Returns cached user directories, resolving them on first use.
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
/// Both paths are relative to a base directory (AppData by default). The
/// destination must be absent or an empty, real directory, even when overwrite
/// is enabled. On failure it may contain partial output; callers must discard
/// staging rather than extract into a live installation. Concurrent mutation
/// of staging by another process is unsupported by the archive crates.
pub async fn extract_file(options: MultiPathOptions, flatten: Option<bool>) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let ifp = full_path(Some(options.init_path), options.init_path_base_dir)
            .context("Could not resolve initial path")?;
        let dfp = full_path(Some(options.dest_path), options.dest_path_base_dir)
            .context("Could not resolve destination path")?;
        let extension = ifp
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .context("The archive has no valid UTF-8 extension")?;
        ensure!(
            matches!(
                extension.as_str(),
                "gz" | "tgz" | "xz" | "txz" | "zst" | "zstd" | "zip"
            ),
            "Unsupported archive extension: .{extension}"
        );
        let file = fs::File::open(&ifp).context("Could not open the initial path")?;
        match fs::symlink_metadata(&dfp) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir(),
                    "Extraction destination must be a real directory"
                );
                ensure!(
                    fs::read_dir(&dfp)?.next().transpose()?.is_none(),
                    "Extraction destination must be empty"
                );
                ensure!(
                    options.overwrite.unwrap_or(true),
                    "Extraction destination already exists"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&dfp).context("Could not create the extraction directory")?;
            }
            Err(error) => return Err(error.into()),
        }
        let root = dfp.canonicalize()?;
        match extension.as_str() {
            "gz" | "tgz" => extract_tar(Gz::new(file), &root)?,
            "xz" | "txz" => extract_tar(Xz::new(file), &root)?,
            "zst" | "zstd" => extract_tar(Zstd::new(file)?, &root)?,
            "zip" => extract_zip(file, &root)?,
            _ => unreachable!("archive extension validated above"),
        }
        validate_archive_links(&root)?;

        if flatten.unwrap_or(true) {
            let entries: Vec<_> = fs::read_dir(&root)?.collect::<Result<_, _>>()?;
            // DirEntry::file_type does not follow a symlink to an outside directory.
            if entries.len() == 1 && entries[0].file_type()?.is_dir() {
                let inner_dir = entries[0].path();
                // Removing the wrapper changes the meaning of relative links.
                validate_archive_links(&inner_dir)?;
                for entry in fs::read_dir(&inner_dir)? {
                    let entry = entry?;
                    fs::rename(entry.path(), root.join(entry.file_name()))?;
                }
                fs::remove_dir(inner_dir)?;
                validate_archive_links(&root)?;
            }
        }
        Ok(())
    })
    .await
    .context("Extraction task panicked or was cancelled")?
}

/// Rejects empty, absolute, or parent-traversing archive entry paths.
fn archive_path(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
        "Unsafe archive path: {}",
        path.display()
    );
    Ok(())
}

/// Extracts TAR entries while keeping paths and link targets inside staging.
fn extract_tar(reader: impl Read, root: &Path) -> Result<()> {
    let mut archive = Tar::new(reader);
    archive.set_overwrite(false);
    archive.set_mask(0o7000);
    let mut directories = Vec::new();
    let mut links = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        // Local PAX/GNU metadata is consumed by entries(); global PAX records
        // have no filesystem node to extract.
        if kind.is_pax_global_extensions() {
            continue;
        }
        archive_path(&entry.path()?)?;
        ensure!(
            kind.is_file()
                || kind.is_gnu_sparse()
                || kind.is_dir()
                || kind.is_symlink()
                || kind.is_hard_link(),
            "Unsupported archive entry type: {kind:?}"
        );
        if kind.is_dir() {
            // Like Archive::unpack, defer restrictive directory permissions.
            fs::create_dir_all(root.join(entry.path()?))?;
            directories.push(entry);
        } else if kind.is_symlink() || kind.is_hard_link() {
            links.push(entry);
        } else {
            ensure!(entry.unpack_in(root)?, "Archive path was not extracted");
        }
    }
    // No file data is ever written through an archive-created symlink. TAR's
    // unpack_in also checks containment of parents and hardlink sources.
    links.sort_by_key(|entry| entry.header().entry_type().is_symlink());
    for mut entry in links {
        let path = entry.path()?.into_owned();
        let target = entry
            .link_name()?
            .context("Archive link has no target")?
            .into_owned();
        ensure!(
            !target.as_os_str().is_empty() && !target.is_absolute(),
            "Archive links must have relative targets"
        );
        resolve_archive_path(root, &path, &mut 40)?;
        if entry.header().entry_type().is_hard_link() {
            let resolved = resolve_archive_path(root, &target, &mut 40)?;
            ensure!(
                fs::symlink_metadata(resolved)?.is_file(),
                "Hardlink target must be an extracted file"
            );
        } else {
            resolve_archive_path(
                root,
                &path.parent().context("Link has no parent")?.join(target),
                &mut 40,
            )?;
        }
        ensure!(entry.unpack_in(root)?, "Archive link was not extracted");
    }
    directories.sort_by_key(|entry| std::cmp::Reverse(entry.path_bytes().into_owned()));
    for mut directory in directories {
        resolve_archive_path(root, &directory.path()?, &mut 40)?;
        ensure!(
            directory.unpack_in(root)?,
            "Archive directory was not extracted"
        );
    }
    Ok(())
}

/// Extracts ZIP entries after checking their paths, types, and link targets.
fn extract_zip(file: fs::File, root: &Path) -> Result<()> {
    let mut archive = Zip::new(file)?;
    // ZipArchive::extract guards writes through existing links but can create
    // an escaping terminal symlink. Validate those targets before extraction.
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().context("Could not decode ZIP entry name")?;
        archive_path(Path::new(name.as_ref()))?;
        let path = entry.enclosed_name().context("Unsafe ZIP entry path")?;
        let kind = entry.unix_mode().unwrap_or(0) & libc::S_IFMT;
        ensure!(
            matches!(kind, 0 | libc::S_IFREG | libc::S_IFDIR | libc::S_IFLNK),
            "Unsupported ZIP entry type"
        );
        if entry.is_symlink() {
            let mut target = String::new();
            (&mut entry).take(4097).read_to_string(&mut target)?;
            ensure!(
                !target.is_empty() && target.len() <= 4096,
                "Invalid ZIP link target length"
            );
            resolve_archive_path(
                root,
                &path.parent().context("Link has no parent")?.join(&target),
                &mut 40,
            )?;
        }
    }
    archive.extract(root)?;
    Ok(())
}

/// Resolve each symlink before processing '..', including dangling internal
/// targets. Lexical normalization alone would miss escapes such as alias/../x.
fn resolve_archive_path(root: &Path, path: &Path, links_left: &mut usize) -> Result<PathBuf> {
    let mut resolved = root.to_path_buf();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                ensure!(
                    resolved != root,
                    "Archive link escapes extraction directory"
                );
                resolved.pop();
            }
            Component::Normal(name) => {
                resolved.push(name);
                match fs::symlink_metadata(&resolved) {
                    Ok(metadata) if metadata.is_symlink() => {
                        ensure!(*links_left > 0, "Archive symlink cycle or excessive depth");
                        *links_left -= 1;
                        let target = fs::read_link(&resolved)?;
                        let parent = resolved
                            .parent()
                            .context("Link has no parent")?
                            .strip_prefix(root)?;
                        resolved = resolve_archive_path(root, &parent.join(target), links_left)?;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            _ => bail!("Archive links must have relative targets"),
        }
    }
    Ok(resolved)
}

/// Checks that every extracted symlink resolves within the extraction root.
fn validate_archive_links(root: &Path) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                resolve_archive_path(root, entry.path().strip_prefix(root)?, &mut 40)?;
            } else if kind.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
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

/// Resolves a configured base directory, defaulting to application data.
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
/// Resolves base directories beneath a temporary profile for tests.
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
    initial_size * 1024.0_f64.powi(initial_unit.power() - new_unit.power())
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

/// Canonicalizes existing ancestors while preserving any missing path suffix.
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

    fn zip_fixture(names: &[&str]) -> (PathBuf, fs::File) {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("elysiae-zip-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("output")).unwrap();
        let mut writer = zip::ZipWriter::new(fs::File::create(root.join("fixture.zip")).unwrap());
        for name in names {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(b"archive contents").unwrap();
        }
        drop(writer.finish().unwrap());
        let file = fs::File::open(root.join("fixture.zip")).unwrap();
        (root, file)
    }

    #[test]
    fn zip_extracts_nested_and_unicode_entry_names() {
        let names = ["nested/data.txt", "nested/日本語.txt"];
        let (root, file) = zip_fixture(&names);
        extract_zip(file, &root.join("output")).unwrap();
        for name in names {
            assert_eq!(
                fs::read(root.join("output").join(name)).unwrap(),
                b"archive contents"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_rejects_unsafe_names_before_extracting_any_files() {
        let absolute =
            std::env::temp_dir().join(format!("elysiae-zip-escape-{}", uuid::Uuid::new_v4()));
        for name in [
            "../escape.txt",
            "nested/../inside.txt",
            "nul\0name.txt",
            absolute.to_str().unwrap(),
        ] {
            let (root, file) = zip_fixture(&["safe.txt", name]);
            let error = extract_zip(file, &root.join("output")).unwrap_err();
            assert!(error.to_string().contains("Unsafe"), "{error:#}");
            assert_eq!(fs::read_dir(root.join("output")).unwrap().count(), 0);
            assert!(!root.join("escape.txt").exists());
            assert!(!absolute.exists());
            fs::remove_dir_all(root).unwrap();
        }
    }

    /// Keeps file paths inside the isolated test profile and rejects traversal.
    #[test]
    fn paths_stay_inside_the_test_profile() {
        for base in [
            None,
            Some(BaseDirectory::Home),
            Some(BaseDirectory::Desktop),
            Some(BaseDirectory::AppData),
            Some(BaseDirectory::Compat),
        ] {
            let root = full_path(None, base).unwrap();
            assert!(root.starts_with(std::env::temp_dir()));
            assert_eq!(
                full_path(Some("game/data".into()), base).unwrap(),
                root.join("game/data")
            );
            for path in ["", ".", "../outside", "/absolute"] {
                assert!(full_path(Some(path.into()), base).is_err());
            }
        }
    }
}
