//! E2 构造不可变发布、验证与重开。

pub mod v2;

use release_store::{ReleaseRoot, ReleaseStore};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tectonics_core::{
    BoundaryComponent, BoundaryEdge, BoundaryKinematicSample, ConstructionMode, Plate,
    TECTONIC_LEVEL, TectonicWorld, validate as validate_tectonic_world,
};
use thiserror::Error;
use world_contracts::{CapabilityState, canonical_json, sha256_hex};

pub const TECTONIC_MANIFEST_SCHEMA: &str = "tectonics-release-v1";
const MANIFEST_FILE: &str = "tectonic-manifest.json";
const MANIFEST_HASH_FILE: &str = "tectonic-manifest.sha256";
const OWNERS_FILE: &str = "owners.u16le";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TectonicProvenance {
    FixedTest,
    RandomPrototype,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TectonicValidationSummary {
    pub profile: String,
    pub status: String,
    pub owner_count: u64,
    pub plate_count: u16,
    pub boundary_edge_count: u64,
    pub boundary_component_count: u64,
    pub kinematic_sample_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TectonicManifest {
    pub schema: String,
    pub tectonic_release_id: String,
    pub world_id: String,
    pub generation_identity: String,
    pub spatial_release_id: String,
    pub spatial_manifest_sha256: String,
    pub model_version: String,
    pub level: u8,
    pub generation_attempt: u8,
    pub provenance: TectonicProvenance,
    pub plates: Vec<Plate>,
    pub boundary_edges: Vec<BoundaryEdge>,
    pub boundary_components: Vec<BoundaryComponent>,
    pub kinematic_samples: Vec<BoundaryKinematicSample>,
    pub owners_sha256: String,
    pub capabilities: BTreeMap<String, CapabilityState>,
    pub validation: TectonicValidationSummary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenedTectonicRelease {
    pub path: PathBuf,
    pub manifest: TectonicManifest,
    pub owners: Vec<u16>,
    pub manifest_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublishedTectonicRelease {
    pub path: PathBuf,
    pub manifest_sha256: String,
    pub owners_sha256: String,
}

#[derive(Debug, Error)]
pub enum TectonicReleaseError {
    #[error("I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("contract failed: {0}")]
    Contract(#[from] world_contracts::ContractError),
    #[error("release store failed: {0}")]
    Store(#[from] release_store::StoreError),
    #[error("invalid tectonic release: {0}")]
    Invalid(String),
    #[error("tectonic release hash mismatch: {0}")]
    HashMismatch(&'static str),
    #[error("tectonic manifest is not canonical JSON")]
    NonCanonicalManifest,
}

impl TectonicManifest {
    #[must_use]
    pub fn from_world(
        release_id: impl Into<String>,
        world: &TectonicWorld,
        spatial_release_id: impl Into<String>,
        spatial_manifest_sha256: impl Into<String>,
    ) -> Self {
        let owners = encode_owners(&world.owners);
        let mut capabilities = BTreeMap::new();
        for name in [
            "main_plate_domains",
            "plate_euler_motion",
            "boundary_normal_qualification",
            "l8_plate_owner",
        ] {
            capabilities.insert(name.to_owned(), CapabilityState::Available);
        }
        for name in [
            "crust_age",
            "elevation",
            "materials",
            "microplates",
            "oblique_classification",
            "strike_slip_classification",
            "subduction_polarity",
            "terrain_generation",
        ] {
            capabilities.insert(name.to_owned(), CapabilityState::NotAvailable);
        }
        Self {
            schema: TECTONIC_MANIFEST_SCHEMA.to_owned(),
            tectonic_release_id: release_id.into(),
            world_id: world.world_id.clone(),
            generation_identity: world.generation_identity.clone(),
            spatial_release_id: spatial_release_id.into(),
            spatial_manifest_sha256: spatial_manifest_sha256.into(),
            model_version: "tectonics-prototype-v1".to_owned(),
            level: world.level,
            generation_attempt: world.generation_attempt,
            provenance: match world.construction_mode {
                ConstructionMode::FixedTest => TectonicProvenance::FixedTest,
                ConstructionMode::RandomPrototype => TectonicProvenance::RandomPrototype,
            },
            plates: world.plates.clone(),
            boundary_edges: world.boundary_edges.clone(),
            boundary_components: world.boundary_components.clone(),
            kinematic_samples: world.kinematic_samples.clone(),
            owners_sha256: sha256_hex(&owners),
            capabilities,
            validation: TectonicValidationSummary {
                profile: "E2-G01..G17-v1".to_owned(),
                status: "PASS".to_owned(),
                owner_count: u64::try_from(world.owners.len()).unwrap_or(u64::MAX),
                plate_count: u16::try_from(world.plates.len()).unwrap_or(u16::MAX),
                boundary_edge_count: u64::try_from(world.boundary_edges.len()).unwrap_or(u64::MAX),
                boundary_component_count: u64::try_from(world.boundary_components.len())
                    .unwrap_or(u64::MAX),
                kinematic_sample_count: u64::try_from(world.kinematic_samples.len())
                    .unwrap_or(u64::MAX),
            },
        }
    }

    /// Validates all reconstructable release invariants.
    ///
    /// # Errors
    /// Returns a stable validation message for malformed metadata or topology.
    pub fn validate(&self, owners: &[u16]) -> Result<(), TectonicReleaseError> {
        validate_release_id(&self.tectonic_release_id)?;
        if self.schema != TECTONIC_MANIFEST_SCHEMA
            || self.model_version != "tectonics-prototype-v1"
            || self.level != TECTONIC_LEVEL
            || self.generation_attempt == 0
            || self.generation_attempt > 8
            || self.world_id.is_empty()
            || self.generation_identity.is_empty()
            || self.spatial_release_id.is_empty()
            || !is_sha256(&self.spatial_manifest_sha256)
            || !is_sha256(&self.owners_sha256)
        {
            return Err(TectonicReleaseError::Invalid(
                "schema, model, level, or attempt is unsupported".into(),
            ));
        }
        let expected_owners = 6_usize * (1_usize << self.level).pow(2);
        if owners.len() != expected_owners
            || self.validation.owner_count != u64::try_from(owners.len()).unwrap_or(u64::MAX)
            || sha256_hex(&encode_owners(owners)) != self.owners_sha256
        {
            return Err(TectonicReleaseError::Invalid(
                "owner field count or digest is invalid".into(),
            ));
        }
        if self.plates.len() != usize::from(self.validation.plate_count)
            || self.boundary_edges.len()
                != usize::try_from(self.validation.boundary_edge_count).unwrap_or(usize::MAX)
            || self.boundary_components.len()
                != usize::try_from(self.validation.boundary_component_count).unwrap_or(usize::MAX)
            || self.kinematic_samples.len()
                != usize::try_from(self.validation.kinematic_sample_count).unwrap_or(usize::MAX)
            || self.boundary_edges.len() != self.kinematic_samples.len()
            || self.validation.status != "PASS"
            || self.validation.profile != "E2-G01..G17-v1"
        {
            return Err(TectonicReleaseError::Invalid(
                "validation counts or status are inconsistent".into(),
            ));
        }
        if owners
            .iter()
            .any(|owner| usize::from(*owner) >= self.plates.len())
        {
            return Err(TectonicReleaseError::Invalid(
                "owner field references an absent plate".into(),
            ));
        }
        let unique_plates = self
            .plates
            .iter()
            .map(|plate| (&plate.plate_id, plate.site_cell_id))
            .collect::<BTreeSet<_>>();
        if unique_plates.len() != self.plates.len() {
            return Err(TectonicReleaseError::Invalid(
                "plate identities or sites are duplicated".into(),
            ));
        }
        validate_capabilities(&self.capabilities)?;
        let world = self.to_world(owners);
        validate_tectonic_world(&world, 6_371_000)
            .map_err(|error| TectonicReleaseError::Invalid(error.to_string()))?;
        Ok(())
    }

    fn to_world(&self, owners: &[u16]) -> TectonicWorld {
        TectonicWorld {
            world_id: self.world_id.clone(),
            generation_identity: self.generation_identity.clone(),
            level: self.level,
            generation_attempt: self.generation_attempt,
            construction_mode: match self.provenance {
                TectonicProvenance::FixedTest => ConstructionMode::FixedTest,
                TectonicProvenance::RandomPrototype => ConstructionMode::RandomPrototype,
            },
            plates: self.plates.clone(),
            owners: owners.to_vec(),
            boundary_edges: self.boundary_edges.clone(),
            boundary_components: self.boundary_components.clone(),
            kinematic_samples: self.kinematic_samples.clone(),
        }
    }
}

fn validate_capabilities(
    capabilities: &BTreeMap<String, CapabilityState>,
) -> Result<(), TectonicReleaseError> {
    for capability in [
        "main_plate_domains",
        "plate_euler_motion",
        "boundary_normal_qualification",
        "l8_plate_owner",
    ] {
        if capabilities.get(capability) != Some(&CapabilityState::Available) {
            return Err(TectonicReleaseError::Invalid(format!(
                "required capability {capability} is absent"
            )));
        }
    }
    for capability in [
        "crust_age",
        "elevation",
        "materials",
        "microplates",
        "oblique_classification",
        "strike_slip_classification",
        "subduction_polarity",
        "terrain_generation",
    ] {
        if capabilities.get(capability) != Some(&CapabilityState::NotAvailable) {
            return Err(TectonicReleaseError::Invalid(format!(
                "deferred capability {capability} is not explicitly unavailable"
            )));
        }
    }
    Ok(())
}

/// Publishes a complete immutable tectonic release.
///
/// # Errors
/// Rejects invalid content, immutable identity collisions, and I/O failures.
pub fn publish(
    store_root: &Path,
    candidate_id: &str,
    manifest: &TectonicManifest,
    owners: &[u16],
) -> Result<PublishedTectonicRelease, TectonicReleaseError> {
    validate_release_id(candidate_id)?;
    manifest.validate(owners)?;
    let store = ReleaseStore::new(store_root)?;
    let candidate = store.create_candidate(candidate_id)?;
    let manifest_bytes = canonical_json(manifest)?;
    let manifest_hash = sha256_hex(&manifest_bytes);
    let owner_bytes = encode_owners(owners);
    fs::write(candidate.join(MANIFEST_FILE), manifest_bytes)?;
    fs::write(candidate.join(MANIFEST_HASH_FILE), manifest_hash.as_bytes())?;
    fs::write(candidate.join(OWNERS_FILE), &owner_bytes)?;
    let path = store.commit(
        &candidate,
        &ReleaseRoot {
            release_id: manifest.tectonic_release_id.clone(),
            asset_ids: Vec::new(),
        },
    )?;
    Ok(PublishedTectonicRelease {
        path,
        manifest_sha256: manifest_hash,
        owners_sha256: sha256_hex(&owner_bytes),
    })
}

/// Opens and verifies an immutable tectonic release.
///
/// # Errors
/// Rejects noncanonical, tampered, truncated, or inconsistent releases.
pub fn open(path: &Path) -> Result<OpenedTectonicRelease, TectonicReleaseError> {
    let manifest_bytes = fs::read(path.join(MANIFEST_FILE))?;
    let manifest_hash = fs::read_to_string(path.join(MANIFEST_HASH_FILE))?;
    if sha256_hex(&manifest_bytes) != manifest_hash {
        return Err(TectonicReleaseError::HashMismatch("manifest"));
    }
    let manifest: TectonicManifest = serde_json::from_slice(&manifest_bytes)?;
    if canonical_json(&manifest)? != manifest_bytes {
        return Err(TectonicReleaseError::NonCanonicalManifest);
    }
    let owners = decode_owners(&fs::read(path.join(OWNERS_FILE))?)?;
    manifest.validate(&owners)?;
    let root: ReleaseRoot = serde_json::from_slice(&fs::read(path.join("release.json"))?)?;
    if root.release_id != manifest.tectonic_release_id || !root.asset_ids.is_empty() {
        return Err(TectonicReleaseError::Invalid(
            "release root disagrees with manifest".into(),
        ));
    }
    Ok(OpenedTectonicRelease {
        path: path.to_owned(),
        manifest,
        owners,
        manifest_sha256: manifest_hash,
    })
}

#[must_use]
pub fn encode_owners(owners: &[u16]) -> Vec<u8> {
    owners
        .iter()
        .flat_map(|owner| owner.to_le_bytes())
        .collect()
}

/// Decodes a canonical owner field.
///
/// # Errors
/// Rejects an odd byte count.
pub fn decode_owners(bytes: &[u8]) -> Result<Vec<u16>, TectonicReleaseError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(TectonicReleaseError::Invalid(
            "owner field has an odd byte count".into(),
        ));
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn validate_release_id(value: &str) -> Result<(), TectonicReleaseError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(TectonicReleaseError::Invalid(
            "release identity contains unsupported characters".into(),
        ));
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tectonics_core::{build_fixed, pv01_fixed_input};
    use world_spec::WorldSpec;

    fn temp_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("terrain-e2-release-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn pv01_publish_open_and_owner_tampering_rejection() {
        let spec = WorldSpec::earthlike("PV01-E2-v1").unwrap();
        let world = build_fixed(&spec, &pv01_fixed_input().unwrap()).unwrap();
        let manifest =
            TectonicManifest::from_world("pv01-release", &world, "e1-fixed", "a".repeat(64));
        let root = temp_path();
        let published = publish(&root, "candidate", &manifest, &world.owners).unwrap();
        let opened = open(&published.path).unwrap();
        assert_eq!(opened.manifest, manifest);
        assert_eq!(opened.owners, world.owners);

        let owner_path = published.path.join(OWNERS_FILE);
        let mut bytes = fs::read(&owner_path).unwrap();
        bytes[0] ^= 0xff;
        fs::write(owner_path, bytes).unwrap();
        assert!(open(&published.path).is_err());
    }

    #[test]
    fn injected_owner_and_boundary_corruption_are_rejected() {
        let spec = WorldSpec::earthlike("PV01-E2-v1").unwrap();
        let world = build_fixed(&spec, &pv01_fixed_input().unwrap()).unwrap();
        let mut manifest =
            TectonicManifest::from_world("pv01-injected", &world, "e1-fixed", "b".repeat(64));
        let mut owners = world.owners.clone();
        owners[0] = u16::MAX;
        manifest.owners_sha256 = sha256_hex(&encode_owners(&owners));
        assert!(manifest.validate(&owners).is_err());

        let mut manifest = TectonicManifest::from_world(
            "pv01-boundary-injected",
            &world,
            "e1-fixed",
            "c".repeat(64),
        );
        manifest.boundary_edges[0].plate_a = manifest.plates[5].plate_id.clone();
        assert!(manifest.validate(&world.owners).is_err());
    }
}
