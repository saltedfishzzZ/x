//! E2.1空间发布v2：保存权威请求，并从请求重建全部补丁声明。

use crate::ReleaseError;
use release_store::{DurabilityGuarantee, ReleaseRoot, ReleaseStore, VisibilityGuarantee};
use serde::{Deserialize, Serialize};
use sphere_core::{CellId, PatchPlan, PatchRequest, SpatialCapability, SpatialReference};
use std::collections::{BTreeMap, BTreeSet};
use world_contracts::{CapabilityState, ReleaseRefV2};
use world_contracts::{canonical_json, sha256_hex};

pub const SPATIAL_RELEASE_SCHEMA_V2: &str = "spatial-release-v2";
pub const REFINEMENT_POLICY_V2: &str = "quadtree-l-plus-2-with-l-plus-1-ring-v2";
pub const HALO_POLICY_V2: &str = "target-resolution-parent-ring-v2";
const MANIFEST_FILE_V2: &str = "spatial-manifest-v2.json";
const HASH_FILE_V2: &str = "spatial-manifest-v2.sha256";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedSpatialReleaseV2 {
    pub path: std::path::PathBuf,
    pub manifest_sha256: String,
    pub visibility: VisibilityGuarantee,
    pub durability: DurabilityGuarantee,
    pub reused_existing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialReferenceV2 {
    pub frame_id: String,
    pub radius_metres: u64,
    pub global_level: u8,
    pub maximum_identity_level: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchDerivedRegionsV2 {
    pub core: Vec<CellId>,
    pub transition: Vec<CellId>,
    pub halo: Vec<CellId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialPatchReleaseV2 {
    pub patch_id: String,
    pub request: PatchRequest,
    pub leaf_cells: Vec<CellId>,
    pub derived_regions: PatchDerivedRegionsV2,
    pub refinement_policy: String,
    pub halo_policy: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialReleaseManifestV2 {
    pub schema: String,
    pub spatial_release_id: String,
    pub identity_format_version: String,
    pub spatial_reference: SpatialReferenceV2,
    pub patches: Vec<SpatialPatchReleaseV2>,
    pub capabilities: BTreeMap<String, CapabilityState>,
    pub direct_dependencies: Vec<ReleaseRefV2>,
}

impl SpatialPatchReleaseV2 {
    #[must_use]
    pub fn from_plan(request: PatchRequest, plan: &PatchPlan) -> Self {
        Self {
            patch_id: plan.patch_id.clone(),
            request,
            leaf_cells: plan.active_leaves().into_iter().collect(),
            derived_regions: PatchDerivedRegionsV2 {
                core: plan.core.iter().copied().collect(),
                transition: plan.transition_leaves.iter().copied().collect(),
                halo: plan.halo.iter().copied().collect(),
            },
            refinement_policy: REFINEMENT_POLICY_V2.to_owned(),
            halo_policy: HALO_POLICY_V2.to_owned(),
        }
    }
}

impl SpatialReleaseManifestV2 {
    /// Builds a v2 manifest from validated spatial facts.
    ///
    /// # Errors
    /// Rejects non-integral reference radii and inconsistent patch inputs.
    pub fn build(
        spatial_release_id: impl Into<String>,
        reference: &SpatialReference,
        mut patches: Vec<(PatchRequest, PatchPlan)>,
        mut direct_dependencies: Vec<ReleaseRefV2>,
    ) -> Result<Self, ReleaseError> {
        if reference.radius_metres.fract() != 0.0
            || reference.radius_metres < 1.0
            || reference.radius_metres > 9_007_199_254_740_992.0
        {
            return Err(ReleaseError::InvalidManifest(
                "v2 spatial reference requires an integral metre radius".to_owned(),
            ));
        }
        patches.sort_by(|left, right| left.1.patch_id.cmp(&right.1.patch_id));
        direct_dependencies.sort();
        let mut capabilities = BTreeMap::new();
        for capability in [
            SpatialCapability::CellIdentity,
            SpatialCapability::CellGeometry,
            SpatialCapability::EdgeGeometry,
            SpatialCapability::NeighbourTopology,
            SpatialCapability::PatchTwoToOne,
            SpatialCapability::PatchQueryStatus,
        ] {
            capabilities.insert(capability.key().to_owned(), CapabilityState::Available);
        }
        let manifest = Self {
            schema: SPATIAL_RELEASE_SCHEMA_V2.to_owned(),
            spatial_release_id: spatial_release_id.into(),
            identity_format_version: "cs1".to_owned(),
            spatial_reference: SpatialReferenceV2 {
                frame_id: reference.frame_id.clone(),
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                radius_metres: reference.radius_metres as u64,
                global_level: reference.global_prototype_level,
                maximum_identity_level: reference.maximum_identity_level,
            },
            patches: patches
                .into_iter()
                .map(|(request, plan)| SpatialPatchReleaseV2::from_plan(request, &plan))
                .collect(),
            capabilities,
            direct_dependencies,
        };
        validate_manifest_v2(&manifest)?;
        Ok(manifest)
    }
}

/// Rebuilds all patch facts and rejects stored claims that differ.
///
/// # Errors
/// Returns an invalid-manifest error with a stable reason prefix.
pub fn validate_manifest_v2(manifest: &SpatialReleaseManifestV2) -> Result<(), ReleaseError> {
    if manifest.schema != SPATIAL_RELEASE_SCHEMA_V2
        || manifest.identity_format_version != "cs1"
        || !safe_id(&manifest.spatial_release_id)
        || manifest.spatial_reference.frame_id != sphere_core::SPATIAL_FRAME_ID
        || manifest.spatial_reference.radius_metres == 0
        || manifest.spatial_reference.global_level
            > manifest.spatial_reference.maximum_identity_level
        || manifest.spatial_reference.maximum_identity_level != 30
    {
        return invalid("SPATIAL_V2_HEADER_INVALID");
    }
    if manifest
        .patches
        .windows(2)
        .any(|pair| pair[0].patch_id >= pair[1].patch_id)
    {
        return invalid("SPATIAL_V2_PATCH_ORDER_INVALID");
    }
    if manifest
        .direct_dependencies
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
        || manifest.direct_dependencies.iter().any(|reference| {
            !safe_id(&reference.release_id) || !sha256_digest(&reference.manifest_sha256)
        })
    {
        return invalid("SPATIAL_V2_DEPENDENCIES_INVALID");
    }
    for capability in [
        SpatialCapability::CellIdentity,
        SpatialCapability::CellGeometry,
        SpatialCapability::EdgeGeometry,
        SpatialCapability::NeighbourTopology,
        SpatialCapability::PatchTwoToOne,
        SpatialCapability::PatchQueryStatus,
    ] {
        if manifest.capabilities.get(capability.key()) != Some(&CapabilityState::Available) {
            return invalid("SPATIAL_V2_CAPABILITY_MISSING");
        }
    }
    for patch in &manifest.patches {
        validate_patch(patch, manifest.spatial_reference.global_level)?;
    }
    Ok(())
}

/// Publishes a fully reconstructed v2 spatial manifest through a sealed store transaction.
///
/// # Errors
/// Rejects invalid manifests, unsafe IDs, candidate conflicts and storage failures.
pub fn publish_v2(
    store_root: &std::path::Path,
    candidate_id: &str,
    manifest: &SpatialReleaseManifestV2,
) -> Result<PublishedSpatialReleaseV2, ReleaseError> {
    validate_manifest_v2(manifest)?;
    let store = ReleaseStore::new(store_root)?;
    let candidate = store.create_candidate_v2(candidate_id, &manifest.spatial_release_id)?;
    let bytes = canonical_json(manifest)?;
    let hash = sha256_hex(&bytes);
    std::fs::write(candidate.join(MANIFEST_FILE_V2), bytes)?;
    std::fs::write(candidate.join(HASH_FILE_V2), hash.as_bytes())?;
    store.mark_candidate_validated(&candidate)?;
    let committed = store.commit_v2(
        &candidate,
        &ReleaseRoot {
            release_id: manifest.spatial_release_id.clone(),
            asset_ids: Vec::new(),
        },
    )?;
    Ok(PublishedSpatialReleaseV2 {
        path: committed.path,
        manifest_sha256: hash,
        visibility: committed.visibility,
        durability: committed.durability,
        reused_existing: committed.reused_existing,
    })
}

/// Opens a v2 spatial release and independently rebuilds every stored patch claim.
///
/// # Errors
/// Rejects missing, tampered, noncanonical or semantically inconsistent releases.
pub fn open_v2(release_path: &std::path::Path) -> Result<SpatialReleaseManifestV2, ReleaseError> {
    let bytes = std::fs::read(release_path.join(MANIFEST_FILE_V2))?;
    let expected_hash = std::fs::read_to_string(release_path.join(HASH_FILE_V2))?;
    if sha256_hex(&bytes) != expected_hash {
        return Err(ReleaseError::HashMismatch);
    }
    let manifest: SpatialReleaseManifestV2 = serde_json::from_slice(&bytes)?;
    if canonical_json(&manifest)? != bytes {
        return Err(ReleaseError::NonCanonicalManifest);
    }
    validate_manifest_v2(&manifest)?;
    let release_root: ReleaseRoot =
        serde_json::from_slice(&std::fs::read(release_path.join("release.json"))?)?;
    if release_root.release_id != manifest.spatial_release_id || !release_root.asset_ids.is_empty()
    {
        return invalid("SPATIAL_V2_RELEASE_ROOT_MISMATCH");
    }
    Ok(manifest)
}

fn validate_patch(patch: &SpatialPatchReleaseV2, global_level: u8) -> Result<(), ReleaseError> {
    if !safe_id(&patch.patch_id)
        || patch.patch_id != patch.request.patch_id
        || patch.refinement_policy != REFINEMENT_POLICY_V2
        || patch.halo_policy != HALO_POLICY_V2
    {
        return invalid("SPATIAL_V2_PATCH_METADATA_INVALID");
    }
    let expected = PatchPlan::build(patch.request.clone())?;
    let expected_leaves: Vec<_> = expected.active_leaves().into_iter().collect();
    if expected.base_level != global_level
        || patch.leaf_cells != expected_leaves
        || patch.derived_regions.core != sorted(&expected.core)
        || patch.derived_regions.transition != sorted(&expected.transition_leaves)
        || patch.derived_regions.halo != sorted(&expected.halo)
    {
        return invalid("SPATIAL_V2_PATCH_RECONSTRUCTION_MISMATCH");
    }
    validate_leaf_set(&patch.leaf_cells)?;
    Ok(())
}

fn validate_leaf_set(leaves: &[CellId]) -> Result<(), ReleaseError> {
    if leaves.windows(2).any(|pair| pair[0] >= pair[1]) {
        return invalid("SPATIAL_V2_LEAF_ORDER_INVALID");
    }
    let set: BTreeSet<_> = leaves.iter().copied().collect();
    for leaf in leaves {
        let mut ancestor = leaf.parent();
        while let Some(parent) = ancestor {
            if set.contains(&parent) {
                return invalid("SPATIAL_V2_LEAF_ANCESTRY_OVERLAP");
            }
            ancestor = parent.parent();
        }
    }
    Ok(())
}

fn sorted(set: &BTreeSet<CellId>) -> Vec<CellId> {
    set.iter().copied().collect()
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn sha256_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid<T>(code: &str) -> Result<T, ReleaseError> {
    Err(ReleaseError::InvalidManifest(code.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sphere_core::{CubeFace, SpatialReference};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("spatial-v2-{name}-{}-{nonce}", std::process::id()))
    }

    fn fixture() -> SpatialReleaseManifestV2 {
        let request = PatchRequest {
            patch_id: "cross-face".to_owned(),
            core_parents: vec![CellId::new(CubeFace::PosX, 8, 255, 128).unwrap()],
            target_level: 10,
            halo_parent_rings: 1,
            cell_budget: 128,
        };
        let plan = PatchPlan::build(request.clone()).unwrap();
        SpatialReleaseManifestV2::build(
            "spatial-v2",
            &SpatialReference::prototype().unwrap(),
            vec![(request, plan)],
            Vec::new(),
        )
        .unwrap()
    }

    #[test]
    fn cross_face_patch_is_reconstructed_from_request() {
        let manifest = fixture();
        assert!(
            manifest.patches[0]
                .derived_regions
                .transition
                .iter()
                .any(|cell| cell.face() != CubeFace::PosX)
        );
        validate_manifest_v2(&manifest).unwrap();
    }

    #[test]
    fn deleted_transition_and_forged_region_are_rejected() {
        let mut deleted = fixture();
        deleted.patches[0].derived_regions.transition.pop();
        assert!(validate_manifest_v2(&deleted).is_err());

        let mut forged = fixture();
        forged.patches[0].derived_regions.halo.clear();
        assert!(validate_manifest_v2(&forged).is_err());
    }

    #[test]
    fn parent_child_leaf_overlap_is_rejected_as_reconstruction_mismatch() {
        let mut manifest = fixture();
        let parent = manifest.patches[0].request.core_parents[0];
        manifest.patches[0].leaf_cells.push(parent);
        manifest.patches[0].leaf_cells.sort_unstable();
        assert!(matches!(
            validate_manifest_v2(&manifest),
            Err(ReleaseError::InvalidManifest(code))
                if code == "SPATIAL_V2_PATCH_RECONSTRUCTION_MISMATCH"
        ));
    }

    #[test]
    fn v2_publish_open_and_tamper_detection_use_the_transaction_store() {
        let root = temp_path("publish");
        let manifest = fixture();
        let published = publish_v2(&root, "candidate", &manifest).unwrap();
        assert_eq!(published.visibility, VisibilityGuarantee::AtomicallyVisible);
        assert_eq!(
            published.durability,
            DurabilityGuarantee::DurabilityNotProven
        );
        assert_eq!(open_v2(&published.path).unwrap(), manifest);
        let path = published.path.join(MANIFEST_FILE_V2);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[10] ^= 1;
        std::fs::write(path, bytes).unwrap();
        assert!(matches!(
            open_v2(&published.path),
            Err(ReleaseError::HashMismatch)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
}
