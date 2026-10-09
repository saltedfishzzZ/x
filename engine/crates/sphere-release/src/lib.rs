//! E1 球面空间发布：规范清单、验证证据、摘要与原子提交。

mod manifest;
pub mod v2;
mod validator;

pub use manifest::{
    IDENTITY_FORMAT_VERSION, SPATIAL_MANIFEST_SCHEMA, SpatialReleaseManifest, TopologyEvidence,
    ValidationEvidence,
};
pub use validator::validate_manifest;

use release_store::{ReleaseRoot, ReleaseStore};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use world_contracts::{canonical_json, sha256_hex};

const MANIFEST_FILE: &str = "spatial-manifest.json";
const HASH_FILE: &str = "spatial-manifest.sha256";

#[derive(Debug, Error)]
pub enum ReleaseError {
    #[error("I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("contract failed: {0}")]
    Contract(#[from] world_contracts::ContractError),
    #[error("release store failed: {0}")]
    Store(#[from] release_store::StoreError),
    #[error("sphere calculation failed: {0}")]
    Sphere(#[from] sphere_core::SphereError),
    #[error("invalid spatial release manifest: {0}")]
    InvalidManifest(String),
    #[error("spatial manifest hash mismatch")]
    HashMismatch,
    #[error("spatial manifest is not in canonical JSON form")]
    NonCanonicalManifest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublishedSpatialRelease {
    pub path: PathBuf,
    pub manifest_sha256: String,
}

/// Validates and atomically publishes an immutable spatial release.
///
/// # Errors
/// Returns a validation, serialization, I/O, or immutable-store error.
pub fn publish(
    store_root: &Path,
    candidate_id: &str,
    manifest: &SpatialReleaseManifest,
) -> Result<PublishedSpatialRelease, ReleaseError> {
    validate_release_id(candidate_id)?;
    validate_manifest(manifest)?;
    let store = ReleaseStore::new(store_root)?;
    let candidate = store.create_candidate(candidate_id)?;
    let bytes = canonical_json(manifest)?;
    let hash = sha256_hex(&bytes);
    fs::write(candidate.join(MANIFEST_FILE), &bytes)?;
    fs::write(candidate.join(HASH_FILE), hash.as_bytes())?;
    let path = store.commit(
        &candidate,
        &ReleaseRoot {
            release_id: manifest.spatial_release_id.clone(),
            asset_ids: Vec::new(),
        },
    )?;
    Ok(PublishedSpatialRelease {
        path,
        manifest_sha256: hash,
    })
}

/// Opens and fully validates a published spatial release directory.
///
/// # Errors
/// Rejects missing, malformed, noncanonical, tampered, or inconsistent data.
pub fn open(release_path: &Path) -> Result<SpatialReleaseManifest, ReleaseError> {
    let bytes = fs::read(release_path.join(MANIFEST_FILE))?;
    let expected_hash = fs::read_to_string(release_path.join(HASH_FILE))?;
    if sha256_hex(&bytes) != expected_hash {
        return Err(ReleaseError::HashMismatch);
    }
    let manifest: SpatialReleaseManifest = serde_json::from_slice(&bytes)?;
    if canonical_json(&manifest)? != bytes {
        return Err(ReleaseError::NonCanonicalManifest);
    }
    validate_manifest(&manifest)?;
    let release_root: ReleaseRoot =
        serde_json::from_slice(&fs::read(release_path.join("release.json"))?)?;
    if release_root.release_id != manifest.spatial_release_id || !release_root.asset_ids.is_empty()
    {
        return Err(ReleaseError::InvalidManifest(
            "release root and spatial manifest disagree".into(),
        ));
    }
    Ok(manifest)
}

pub(crate) fn validate_release_id(value: &str) -> Result<(), ReleaseError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ReleaseError::InvalidManifest(
            "release identity contains unsupported characters".into(),
        ));
    }
    Ok(())
}
