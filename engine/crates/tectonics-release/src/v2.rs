//! E2.1构造发布v2及事务化重开验证。

use crate::TectonicReleaseError;
use release_store::{
    DurabilityGuarantee, ReleaseRoot, ReleaseStore, SafeStoreId, VisibilityGuarantee,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tectonics_core::TECTONIC_LEVEL;
use tectonics_core::v2::{TectonicKinematicsV2, TectonicSkeletonV2};
use world_contracts::{CapabilityState, ReleaseRefV2, canonical_json, sha256_hex};
use world_spec::v2::GenerationIdentityV2;

pub const TECTONIC_RELEASE_SCHEMA_V2: &str = "tectonics-release-v2";
const MANIFEST_FILE_V2: &str = "tectonic-manifest-v2.json";
const HASH_FILE_V2: &str = "tectonic-manifest-v2.sha256";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TectonicManifestV2 {
    pub schema: String,
    pub tectonic_release_id: String,
    pub world_id: String,
    pub generation_identity: String,
    pub spatial_release: ReleaseRefV2,
    pub skeleton: TectonicSkeletonV2,
    pub kinematics: TectonicKinematicsV2,
    pub capabilities: BTreeMap<String, CapabilityState>,
    pub direct_dependencies: Vec<ReleaseRefV2>,
    pub validation_passed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedTectonicReleaseV2 {
    pub path: PathBuf,
    pub manifest_sha256: String,
    pub visibility: VisibilityGuarantee,
    pub durability: DurabilityGuarantee,
    pub reused_existing: bool,
}

impl TectonicManifestV2 {
    /// Builds a v2 release manifest from already generated skeleton and kinematics.
    ///
    /// # Errors
    /// Rejects mismatched generation, spatial, skeleton, motion or ownership state.
    pub fn build(
        tectonic_release_id: impl Into<String>,
        generation: &GenerationIdentityV2,
        spatial_release: ReleaseRefV2,
        skeleton: TectonicSkeletonV2,
        kinematics: TectonicKinematicsV2,
    ) -> Result<Self, TectonicReleaseError> {
        let mut capabilities = BTreeMap::new();
        for name in [
            "main_plate_domains",
            "plate_euler_motion",
            "boundary_direction_type",
            "boundary_activity_strength",
            "l8_plate_owner",
        ] {
            capabilities.insert(name.to_owned(), CapabilityState::Available);
        }
        let manifest = Self {
            schema: TECTONIC_RELEASE_SCHEMA_V2.to_owned(),
            tectonic_release_id: tectonic_release_id.into(),
            world_id: generation.world_id.clone(),
            generation_identity: generation.generation_identity.clone(),
            direct_dependencies: vec![spatial_release.clone()],
            spatial_release,
            skeleton,
            kinematics,
            capabilities,
            validation_passed: true,
        };
        validate_manifest_v2(&manifest)?;
        Ok(manifest)
    }
}

/// Revalidates all reconstructable v2 tectonic release invariants.
///
/// # Errors
/// Returns stable invalid-release reason codes.
pub fn validate_manifest_v2(manifest: &TectonicManifestV2) -> Result<(), TectonicReleaseError> {
    SafeStoreId::parse(&manifest.tectonic_release_id)?;
    if manifest.schema != TECTONIC_RELEASE_SCHEMA_V2
        || manifest.world_id.is_empty()
        || manifest.generation_identity.is_empty()
        || manifest.kinematics.generation_identity != manifest.generation_identity
        || manifest.kinematics.tectonic_skeleton_id != manifest.skeleton.tectonic_skeleton_id
        || manifest.skeleton.level != TECTONIC_LEVEL
        || manifest.skeleton.generation_attempt == 0
        || !manifest.validation_passed
    {
        return invalid("TECTONIC_V2_IDENTITY_MISMATCH");
    }
    if manifest.direct_dependencies != [manifest.spatial_release.clone()]
        || !valid_reference(&manifest.spatial_release)
    {
        return invalid("TECTONIC_V2_DEPENDENCY_MISMATCH");
    }
    let expected_owners = 6_usize * (1_usize << manifest.skeleton.level).pow(2);
    if manifest.skeleton.owners.len() != expected_owners
        || manifest.skeleton.plate_roles.is_empty()
        || manifest
            .skeleton
            .owners
            .iter()
            .any(|owner| usize::from(*owner) >= manifest.skeleton.plate_roles.len())
    {
        return invalid("TECTONIC_V2_OWNER_FIELD_INVALID");
    }
    if manifest.kinematics.plate_motions.len() != manifest.skeleton.plate_roles.len()
        || manifest
            .kinematics
            .plate_motions
            .iter()
            .zip(&manifest.skeleton.plate_roles)
            .any(|(motion, role)| motion.plate_role_id != role.plate_role_id)
        || manifest
            .kinematics
            .boundary_samples
            .windows(2)
            .any(|pair| pair[0].edge_id >= pair[1].edge_id)
    {
        return invalid("TECTONIC_V2_KINEMATICS_INVALID");
    }
    for capability in [
        "main_plate_domains",
        "plate_euler_motion",
        "boundary_direction_type",
        "boundary_activity_strength",
        "l8_plate_owner",
    ] {
        if manifest.capabilities.get(capability) != Some(&CapabilityState::Available) {
            return invalid("TECTONIC_V2_CAPABILITY_MISSING");
        }
    }
    Ok(())
}

/// Publishes a validated v2 tectonic release through the v2 transaction store.
///
/// # Errors
/// Rejects invalid content, unsafe IDs, conflicts and I/O failures.
pub fn publish_v2(
    store_root: &Path,
    candidate_id: &str,
    manifest: &TectonicManifestV2,
) -> Result<PublishedTectonicReleaseV2, TectonicReleaseError> {
    validate_manifest_v2(manifest)?;
    let store = ReleaseStore::new(store_root)?;
    let candidate = store.create_candidate_v2(candidate_id, &manifest.tectonic_release_id)?;
    let bytes = canonical_json(manifest)?;
    let hash = sha256_hex(&bytes);
    std::fs::write(candidate.join(MANIFEST_FILE_V2), bytes)?;
    std::fs::write(candidate.join(HASH_FILE_V2), hash.as_bytes())?;
    store.mark_candidate_validated(&candidate)?;
    let committed = store.commit_v2(
        &candidate,
        &ReleaseRoot {
            release_id: manifest.tectonic_release_id.clone(),
            asset_ids: Vec::new(),
        },
    )?;
    Ok(PublishedTectonicReleaseV2 {
        path: committed.path,
        manifest_sha256: hash,
        visibility: committed.visibility,
        durability: committed.durability,
        reused_existing: committed.reused_existing,
    })
}

/// Opens and fully validates a v2 tectonic release directory.
///
/// # Errors
/// Rejects missing, tampered, noncanonical or inconsistent releases.
pub fn open_v2(path: &Path) -> Result<TectonicManifestV2, TectonicReleaseError> {
    let bytes = std::fs::read(path.join(MANIFEST_FILE_V2))?;
    let expected_hash = std::fs::read_to_string(path.join(HASH_FILE_V2))?;
    if sha256_hex(&bytes) != expected_hash {
        return Err(TectonicReleaseError::HashMismatch("manifest-v2"));
    }
    let manifest: TectonicManifestV2 = serde_json::from_slice(&bytes)?;
    if canonical_json(&manifest)? != bytes {
        return Err(TectonicReleaseError::NonCanonicalManifest);
    }
    validate_manifest_v2(&manifest)?;
    let root: ReleaseRoot = serde_json::from_slice(&std::fs::read(path.join("release.json"))?)?;
    if root.release_id != manifest.tectonic_release_id || !root.asset_ids.is_empty() {
        return invalid("TECTONIC_V2_RELEASE_ROOT_MISMATCH");
    }
    Ok(manifest)
}

fn valid_reference(reference: &ReleaseRefV2) -> bool {
    SafeStoreId::parse(&reference.release_id).is_ok()
        && reference.manifest_sha256.len() == 64
        && reference
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid<T>(code: &str) -> Result<T, TectonicReleaseError> {
    Err(TectonicReleaseError::Invalid(code.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tectonics_core::v2::{build_kinematics, build_natural_skeleton};
    use world_spec::v2::{GenerationSpecV2, WorldIntentSpecV2};

    fn temp_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tectonic-v2-{name}-{}-{nonce}", std::process::id()))
    }

    fn fixture() -> TectonicManifestV2 {
        let intent = WorldIntentSpecV2::earthlike_natural("tectonic-release-v2").unwrap();
        let spatial = ReleaseRefV2 {
            release_id: "spatial-v2".to_owned(),
            manifest_sha256: "a".repeat(64),
        };
        let spec = GenerationSpecV2::new(
            &intent,
            spatial.clone(),
            BTreeMap::from([("tectonics".to_owned(), "tectonics-v2".to_owned())]),
        )
        .unwrap();
        let generation = spec.identity(&intent).unwrap();
        let skeleton = build_natural_skeleton(&intent).unwrap();
        let kinematics = build_kinematics(&intent, &generation, &skeleton).unwrap();
        TectonicManifestV2::build("tectonic-v2", &generation, spatial, skeleton, kinematics)
            .unwrap()
    }

    #[test]
    fn v2_publish_open_and_tamper_detection() {
        let root = temp_path("publish");
        let manifest = fixture();
        let published = publish_v2(&root, "candidate", &manifest).unwrap();
        assert_eq!(open_v2(&published.path).unwrap(), manifest);
        let path = published.path.join(MANIFEST_FILE_V2);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[20] ^= 1;
        std::fs::write(path, bytes).unwrap();
        assert!(matches!(
            open_v2(&published.path),
            Err(TectonicReleaseError::HashMismatch("manifest-v2"))
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forged_owner_and_admission_flag_are_rejected() {
        let mut owner = fixture();
        owner.skeleton.owners[0] = u16::MAX;
        assert!(validate_manifest_v2(&owner).is_err());
        let mut flag = fixture();
        flag.validation_passed = false;
        assert!(validate_manifest_v2(&flag).is_err());
    }
}
