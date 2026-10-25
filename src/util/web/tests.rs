use super::*;

/// Allows HTTPS requests while rejecting insecure and local-file URLs.
#[test]
fn requests_require_https() {
    assert!(https_url("https://example.invalid/file").is_ok());

    for url in [
        "http://example.invalid/file",
        "ftp://example.invalid/file",
        "file:///tmp/file",
    ] {
        assert!(https_url(url).is_err());
    }
}
