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
    let home = full_path(None, Some(BaseDirectory::Home)).unwrap();
    assert_eq!(
        full_path(None, None).unwrap(),
        home.join(".local/share/elysiae")
    );

    for base in [
        None,
        Some(BaseDirectory::Home),
        Some(BaseDirectory::Desktop),
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
