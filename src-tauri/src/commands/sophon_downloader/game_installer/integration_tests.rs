use reqwest::Client;

use super::super::compute_content_manifest_hash;
use super::api::{fetch_build, fetch_front_door, fetch_manifest};

#[tokio::test]
#[ignore]
async fn fetch_front_door_returns_valid_response() {
    let client = Client::new();
    let (branch, _pre) = fetch_front_door(&client, "hk4e")
        .await
        .expect("fetch_front_door should succeed");
    assert!(
        !branch.main.password.is_empty(),
        "branch password should be non-empty"
    );
    assert!(
        !branch.main.package_id.is_empty(),
        "package_id should be non-empty"
    );
}

#[tokio::test]
#[ignore]
async fn fetch_build_returns_manifests() {
    let client = Client::new();
    let (branch, _) = fetch_front_door(&client, "hk4e")
        .await
        .expect("fetch_front_door should succeed");
    let build = fetch_build(&client, &branch.main, None)
        .await
        .expect("fetch_build should succeed");
    assert!(
        !build.manifests.is_empty(),
        "build data should contain manifests"
    );
}

#[tokio::test]
#[ignore]
async fn fetch_manifest_decodes_successfully() {
    let client = Client::new();
    let (branch, _) = fetch_front_door(&client, "hk4e")
        .await
        .expect("fetch_front_door should succeed");
    let build = fetch_build(&client, &branch.main, None)
        .await
        .expect("fetch_build should succeed");
    let meta = &build.manifests[0];
    let result = fetch_manifest(&client, &meta.manifest_download, &meta.manifest.id)
        .await
        .expect("fetch_manifest should succeed");
    assert!(
        !result.manifest.assets.is_empty(),
        "manifest should contain assets"
    );
}

#[tokio::test]
#[ignore]
async fn decode_manifest_with_real_data() {
    let client = Client::new();
    let (branch, _) = fetch_front_door(&client, "hk4e")
        .await
        .expect("fetch_front_door should succeed");
    let build = fetch_build(&client, &branch.main, None)
        .await
        .expect("fetch_build should succeed");
    let meta = &build.manifests[0];
    let result = fetch_manifest(&client, &meta.manifest_download, &meta.manifest.id)
        .await
        .expect("fetch_manifest should succeed");
    for asset in &result.manifest.assets {
        if !asset.is_directory() {
            assert!(
                !asset.asset_hash_md5.is_empty(),
                "non-directory asset should have non-empty asset_hash_md5"
            );
        }
    }
}

#[tokio::test]
#[ignore]
async fn download_chunk_from_cdn() {
    let client = Client::new();
    let (branch, _) = fetch_front_door(&client, "hk4e")
        .await
        .expect("fetch_front_door should succeed");
    let build = fetch_build(&client, &branch.main, None)
        .await
        .expect("fetch_build should succeed");
    let meta = &build.manifests[0];
    let result = fetch_manifest(&client, &meta.manifest_download, &meta.manifest.id)
        .await
        .expect("fetch_manifest should succeed");
    let first_asset = result
        .manifest
        .assets
        .iter()
        .find(|a| !a.asset_chunks.is_empty())
        .expect("should find an asset with chunks");
    let first_chunk = &first_asset.asset_chunks[0];
    let url = meta.chunk_download.url_for(&first_chunk.chunk_name);
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .expect("chunk request should succeed");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::OK,
        "CDN should respond with 200"
    );
    let _bytes = resp
        .bytes()
        .await
        .expect("should be able to read response body");
}

#[tokio::test]
#[ignore]
async fn compute_content_manifest_hash_with_real_manifest() {
    let client = Client::new();
    let (branch, _) = fetch_front_door(&client, "hk4e")
        .await
        .expect("fetch_front_door should succeed");
    let build = fetch_build(&client, &branch.main, None)
        .await
        .expect("fetch_build should succeed");
    let meta = &build.manifests[0];
    let result = fetch_manifest(&client, &meta.manifest_download, &meta.manifest.id)
        .await
        .expect("fetch_manifest should succeed");
    let hash = compute_content_manifest_hash(&result.manifest);
    assert_eq!(
        hash.len(),
        16,
        "content manifest hash should be 16 hex chars"
    );
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should be hex"
    );
}

#[tokio::test]
#[ignore]
async fn front_door_all_known_games() {
    let client = Client::new();
    let game_ids = ["nap", "hkrpg", "hk4e", "bh3"];
    for game_id in game_ids {
        let (branch, _) = fetch_front_door(&client, game_id)
            .await
            .expect("fetch_front_door should succeed for {game_id}");
        assert!(
            !branch.main.password.is_empty(),
            "branch password should be non-empty for {game_id}"
        );
        assert!(
            !branch.main.package_id.is_empty(),
            "package_id should be non-empty for {game_id}"
        );
    }
}
