use sphere_core::{CellId, CubeFace, PatchPlan, PatchRequest, SpatialReference};
use sphere_release::{ReleaseError, SpatialReleaseManifest, open, publish, validate_manifest};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("e1-{name}-{}-{nonce}", std::process::id()))
}

fn manifest(release_id: &str) -> SpatialReleaseManifest {
    let parent = CellId::new(CubeFace::PosX, 8, 120, 130).unwrap();
    let patch = PatchPlan::build(PatchRequest {
        patch_id: "E1F06".into(),
        core_parents: vec![parent],
        target_level: 10,
        halo_parent_rings: 1,
        cell_budget: 128,
    })
    .unwrap();
    SpatialReleaseManifest::e1(
        release_id,
        SpatialReference::prototype().unwrap(),
        vec![patch],
    )
    .unwrap()
}

#[test]
fn e1_g14_publish_open_and_immutable_identity() {
    let root = temp_path("publish");
    let source = manifest("spatial-e1");
    let published = publish(&root, "candidate-a", &source).unwrap();
    assert_eq!(open(&published.path).unwrap(), source);
    assert!(matches!(
        publish(&root, "candidate-b", &source),
        Err(ReleaseError::Store(
            release_store::StoreError::ReleaseExists
        ))
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn e1_g13_injected_cross_face_corruption_is_rejected() {
    let mut source = manifest("spatial-pi01");
    source.validation_evidence.l0_topology[0].target_cell =
        CellId::new(CubeFace::NegX, 0, 0, 0).unwrap();
    assert!(matches!(
        validate_manifest(&source),
        Err(ReleaseError::InvalidManifest(_))
    ));
}

#[test]
fn e1_g14_manifest_byte_tampering_is_rejected() {
    let root = temp_path("tamper");
    let published = publish(&root, "candidate", &manifest("spatial-tamper")).unwrap();
    let path = published.path.join("spatial-manifest.json");
    let mut bytes = fs::read(&path).unwrap();
    bytes[10] ^= 1;
    fs::write(path, bytes).unwrap();
    assert!(matches!(
        open(&published.path),
        Err(ReleaseError::HashMismatch)
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn e1_g14_noncanonical_manifest_is_rejected_even_with_matching_hash() {
    let root = temp_path("noncanonical");
    let published = publish(&root, "candidate", &manifest("spatial-format")).unwrap();
    let path = published.path.join("spatial-manifest.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let pretty = serde_json::to_vec_pretty(&value).unwrap();
    fs::write(&path, &pretty).unwrap();
    fs::write(
        published.path.join("spatial-manifest.sha256"),
        world_contracts::sha256_hex(&pretty),
    )
    .unwrap();
    assert!(matches!(
        open(&published.path),
        Err(ReleaseError::NonCanonicalManifest)
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn e1_f08_rejects_path_like_release_identity() {
    let source = manifest("../escape");
    assert!(matches!(
        validate_manifest(&source),
        Err(ReleaseError::InvalidManifest(_))
    ));
}
