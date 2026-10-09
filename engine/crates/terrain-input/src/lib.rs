//! E2 M1 输入、运行计划、垂直基准和准入。

use release_store::{ReleaseRoot, ReleaseStore};
use serde::{Deserialize, Serialize};
use sphere_release::SpatialReleaseManifest;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tectonics_release::{TectonicManifest, TectonicProvenance};
use thiserror::Error;
use world_contracts::{CapabilityState, canonical_json, sha256_hex};
use world_spec::{DEFAULT_RADIUS_METRES, RANDOM_PROTOCOL, WorldSpec, WorldSpecError};

pub mod v2;

pub const TERRAIN_INPUT_SCHEMA: &str = "terrain-input-v1";
const RELEASE_FILE: &str = "m1-input-release.json";
const HASH_FILE: &str = "m1-input-release.sha256";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerticalDatumDescriptor {
    pub datum_id: String,
    pub origin: String,
    pub radial_direction_source: String,
    pub reference_radius_metres: u64,
    pub elevation_unit: String,
    pub zero_surface: String,
    pub sea_level_relation: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainMaterialInputBundle {
    pub schema: String,
    pub input_bundle_id: String,
    pub generation_identity: String,
    pub world_id: String,
    pub world_spec_sha256: String,
    pub spatial_release_id: String,
    pub spatial_manifest_sha256: String,
    pub tectonic_release_id: String,
    pub tectonic_manifest_sha256: String,
    pub terrain_material_model: String,
    pub parameter_profile: String,
    pub vertical_datum: VerticalDatumDescriptor,
    pub random_protocol: String,
    pub random_namespaces: Vec<String>,
    pub required_capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunProfile {
    FixedTest,
    Diagnostic,
    StandardGlobal,
    RegionalRefinement,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerrainMaterialRunPlan {
    pub run_plan_id: String,
    pub profile: RunProfile,
    pub output_namespace: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdmissionConclusion {
    Admitted,
    DiagnosticOnly,
    Rejected,
    Unresolved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionReason {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionReport {
    pub input_bundle_id: String,
    pub run_plan_id: String,
    pub conclusion: AdmissionConclusion,
    pub reasons: Vec<AdmissionReason>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M1InputRelease {
    pub schema: String,
    pub m1_release_id: String,
    pub input_bundle: TerrainMaterialInputBundle,
    pub run_plan: TerrainMaterialRunPlan,
    pub admission_report: AdmissionReport,
    pub capabilities: BTreeMap<String, CapabilityState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublishedM1InputRelease {
    pub path: PathBuf,
    pub release_sha256: String,
}

#[derive(Debug, Error)]
pub enum TerrainInputError {
    #[error("world specification failed: {0}")]
    World(#[from] WorldSpecError),
    #[error("contract failed: {0}")]
    Contract(#[from] world_contracts::ContractError),
    #[error("release store failed: {0}")]
    Store(#[from] release_store::StoreError),
    #[error("I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid M1 input: {0}")]
    Invalid(String),
    #[error("M1 release hash mismatch")]
    HashMismatch,
    #[error("M1 release is not canonical JSON")]
    NonCanonical,
}

impl TerrainMaterialInputBundle {
    /// Builds a closed M1 input bundle from verified upstream manifests.
    ///
    /// # Errors
    /// Rejects identity, dependency, radius, or capability mismatches.
    pub fn build(
        spec: &WorldSpec,
        spatial: &SpatialReleaseManifest,
        spatial_manifest_sha256: impl Into<String>,
        tectonic: &TectonicManifest,
        tectonic_manifest_sha256: impl Into<String>,
    ) -> Result<Self, TerrainInputError> {
        let identity = spec.identity()?;
        let spatial_hash = spatial_manifest_sha256.into();
        let tectonic_hash = tectonic_manifest_sha256.into();
        if identity.world_id != tectonic.world_id
            || identity.generation_identity != tectonic.generation_identity
            || tectonic.spatial_release_id != spatial.spatial_release_id
            || tectonic.spatial_manifest_sha256 != spatial_hash
            || spec.radius_metres != DEFAULT_RADIUS_METRES
            || spatial.spatial_reference.radius_metres.to_bits()
                != f64::from(u32::try_from(DEFAULT_RADIUS_METRES).unwrap_or(u32::MAX)).to_bits()
        {
            return Err(TerrainInputError::Invalid(
                "world, spatial, tectonic, or vertical datum identity mismatch".into(),
            ));
        }
        for capability in [
            "main_plate_domains",
            "plate_euler_motion",
            "boundary_normal_qualification",
            "l8_plate_owner",
        ] {
            if tectonic.capabilities.get(capability) != Some(&CapabilityState::Available) {
                return Err(TerrainInputError::Invalid(format!(
                    "required tectonic capability {capability} is unavailable"
                )));
            }
        }
        let spec_bytes = canonical_json(spec)?;
        let world_spec_sha256 = sha256_hex(&spec_bytes);
        let identity_bytes = canonical_json(&(
            &identity.generation_identity,
            &world_spec_sha256,
            &spatial_hash,
            &tectonic_hash,
            "terrain-material-prototype-v1",
            "earthlike-prototype-v1",
            RANDOM_PROTOCOL,
        ))?;
        let mut bundle = Self {
            schema: TERRAIN_INPUT_SCHEMA.to_owned(),
            input_bundle_id: String::new(),
            generation_identity: identity.generation_identity,
            world_id: identity.world_id,
            world_spec_sha256,
            spatial_release_id: spatial.spatial_release_id.clone(),
            spatial_manifest_sha256: spatial_hash,
            tectonic_release_id: tectonic.tectonic_release_id.clone(),
            tectonic_manifest_sha256: tectonic_hash,
            terrain_material_model: "terrain-material-prototype-v1".to_owned(),
            parameter_profile: "earthlike-prototype-v1".to_owned(),
            vertical_datum: VerticalDatumDescriptor {
                datum_id: "reference-sphere-v1".to_owned(),
                origin: "planet-centre".to_owned(),
                radial_direction_source: "E1 authoritative UnitVec3".to_owned(),
                reference_radius_metres: DEFAULT_RADIUS_METRES,
                elevation_unit: "m".to_owned(),
                zero_surface: "reference-sphere".to_owned(),
                sea_level_relation: "SEPARATE_NOT_AVAILABLE".to_owned(),
            },
            random_protocol: RANDOM_PROTOCOL.to_owned(),
            random_namespaces: vec![
                "materials".to_owned(),
                "terrain".to_owned(),
                "terrain-detail".to_owned(),
            ],
            required_capabilities: vec![
                "boundary_normal_qualification".to_owned(),
                "l8_plate_owner".to_owned(),
                "main_plate_domains".to_owned(),
                "plate_euler_motion".to_owned(),
            ],
        };
        bundle.input_bundle_id = format!("input-{}", &sha256_hex(&identity_bytes)[..24]);
        Ok(bundle)
    }
}

impl TerrainMaterialRunPlan {
    /// Creates a content-addressed run plan.
    ///
    /// # Errors
    /// Rejects an empty or path-like output namespace.
    pub fn new(profile: RunProfile, output_namespace: &str) -> Result<Self, TerrainInputError> {
        if output_namespace.is_empty()
            || output_namespace.starts_with('/')
            || output_namespace.contains("..")
        {
            return Err(TerrainInputError::Invalid(
                "output namespace is empty or path-like".into(),
            ));
        }
        let bytes = canonical_json(&(profile, output_namespace))?;
        Ok(Self {
            run_plan_id: format!("run-plan-{}", &sha256_hex(&bytes)[..24]),
            profile,
            output_namespace: output_namespace.to_owned(),
        })
    }
}

/// Evaluates E2 M1 admission without altering the bundle or run plan.
#[must_use]
pub fn admit(
    bundle: &TerrainMaterialInputBundle,
    plan: &TerrainMaterialRunPlan,
    tectonic: &TectonicManifest,
) -> AdmissionReport {
    let (conclusion, code, detail) = if tectonic.validation.status == "PASS" {
        match plan.profile {
            RunProfile::FixedTest
                if tectonic.provenance == TectonicProvenance::FixedTest
                    && plan.output_namespace.starts_with("test/") =>
            {
                (
                    AdmissionConclusion::Admitted,
                    "FIXED_TEST_ADMITTED",
                    "固定测试输入获准进入测试命名空间",
                )
            }
            RunProfile::FixedTest => (
                AdmissionConclusion::Rejected,
                "FIXED_TEST_ISOLATION_FAILED",
                "固定测试要求固定来源和test/命名空间",
            ),
            RunProfile::Diagnostic => (
                AdmissionConclusion::DiagnosticOnly,
                "PROTOTYPE_DIAGNOSTIC_ONLY",
                "普通E2构造仅允许诊断",
            ),
            RunProfile::StandardGlobal => (
                AdmissionConclusion::Rejected,
                "RUN_PROFILE_NOT_AVAILABLE",
                "STANDARD_GLOBAL等待E3补齐L5/L6消费字段",
            ),
            RunProfile::RegionalRefinement => (
                AdmissionConclusion::Rejected,
                "RUN_PROFILE_NOT_AVAILABLE",
                "REGIONAL_REFINEMENT在E2不可用",
            ),
        }
    } else {
        (
            AdmissionConclusion::Unresolved,
            "TECTONIC_EVIDENCE_INCOMPLETE",
            "构造验证证据不是PASS",
        )
    };
    AdmissionReport {
        input_bundle_id: bundle.input_bundle_id.clone(),
        run_plan_id: plan.run_plan_id.clone(),
        conclusion,
        reasons: vec![AdmissionReason {
            code: code.to_owned(),
            detail: detail.to_owned(),
        }],
    }
}

impl M1InputRelease {
    #[must_use]
    pub fn new(
        release_id: impl Into<String>,
        bundle: TerrainMaterialInputBundle,
        run_plan: TerrainMaterialRunPlan,
        tectonic: &TectonicManifest,
    ) -> Self {
        let admission_report = admit(&bundle, &run_plan, tectonic);
        let mut capabilities = BTreeMap::new();
        capabilities.insert("fixed_test".to_owned(), CapabilityState::Available);
        capabilities.insert("diagnostic".to_owned(), CapabilityState::Available);
        capabilities.insert("standard_global".to_owned(), CapabilityState::NotAvailable);
        capabilities.insert(
            "regional_refinement".to_owned(),
            CapabilityState::NotAvailable,
        );
        for surface in ["solid_surface", "water_surface", "snow_surface"] {
            capabilities.insert(surface.to_owned(), CapabilityState::NotAvailable);
        }
        Self {
            schema: TERRAIN_INPUT_SCHEMA.to_owned(),
            m1_release_id: release_id.into(),
            input_bundle: bundle,
            run_plan,
            admission_report,
            capabilities,
        }
    }

    /// Validates cross-references and capability/admission consistency.
    ///
    /// # Errors
    /// Rejects an inconsistent or unsupported M1 release.
    pub fn validate(&self) -> Result<(), TerrainInputError> {
        validate_id(&self.m1_release_id)?;
        if self.schema != TERRAIN_INPUT_SCHEMA
            || self.admission_report.input_bundle_id != self.input_bundle.input_bundle_id
            || self.admission_report.run_plan_id != self.run_plan.run_plan_id
        {
            return Err(TerrainInputError::Invalid(
                "M1 schema or atomic cross-reference mismatch".into(),
            ));
        }
        if matches!(self.run_plan.profile, RunProfile::StandardGlobal)
            && self.admission_report.conclusion != AdmissionConclusion::Rejected
        {
            return Err(TerrainInputError::Invalid(
                "STANDARD_GLOBAL cannot be admitted in E2".into(),
            ));
        }
        Ok(())
    }
}

/// Atomically publishes one M1 input release.
///
/// # Errors
/// Rejects validation, I/O, and immutable identity failures.
pub fn publish(
    store_root: &Path,
    candidate_id: &str,
    release: &M1InputRelease,
) -> Result<PublishedM1InputRelease, TerrainInputError> {
    validate_id(candidate_id)?;
    release.validate()?;
    let store = ReleaseStore::new(store_root)?;
    let candidate = store.create_candidate(candidate_id)?;
    let bytes = canonical_json(release)?;
    let hash = sha256_hex(&bytes);
    fs::write(candidate.join(RELEASE_FILE), bytes)?;
    fs::write(candidate.join(HASH_FILE), hash.as_bytes())?;
    let path = store.commit(
        &candidate,
        &ReleaseRoot {
            release_id: release.m1_release_id.clone(),
            asset_ids: Vec::new(),
        },
    )?;
    Ok(PublishedM1InputRelease {
        path,
        release_sha256: hash,
    })
}

/// Opens and validates one M1 input release.
///
/// # Errors
/// Rejects noncanonical, tampered, or inconsistent releases.
pub fn open(path: &Path) -> Result<M1InputRelease, TerrainInputError> {
    let bytes = fs::read(path.join(RELEASE_FILE))?;
    if sha256_hex(&bytes) != fs::read_to_string(path.join(HASH_FILE))? {
        return Err(TerrainInputError::HashMismatch);
    }
    let release: M1InputRelease = serde_json::from_slice(&bytes)?;
    if canonical_json(&release)? != bytes {
        return Err(TerrainInputError::NonCanonical);
    }
    release.validate()?;
    let root: ReleaseRoot = serde_json::from_slice(&fs::read(path.join("release.json"))?)?;
    if root.release_id != release.m1_release_id || !root.asset_ids.is_empty() {
        return Err(TerrainInputError::Invalid(
            "release root disagrees with M1 release".into(),
        ));
    }
    Ok(release)
}

fn validate_id(value: &str) -> Result<(), TerrainInputError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(TerrainInputError::Invalid(
            "identity contains unsupported characters".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sphere_core::SpatialReference;
    use tectonics_core::{build_fixed, pv01_fixed_input};
    use tectonics_release::TectonicManifest;

    #[test]
    fn run_plan_is_separate_from_generation_identity() {
        let diagnostic =
            TerrainMaterialRunPlan::new(RunProfile::Diagnostic, "diagnostic/a").unwrap();
        let standard =
            TerrainMaterialRunPlan::new(RunProfile::StandardGlobal, "release/a").unwrap();
        assert_ne!(diagnostic.run_plan_id, standard.run_plan_id);
    }

    #[test]
    fn path_like_namespace_is_rejected() {
        assert!(TerrainMaterialRunPlan::new(RunProfile::Diagnostic, "../escape").is_err());
    }

    #[test]
    fn e2_admission_matrix_and_oracle_isolation_hold() {
        let spec = WorldSpec::earthlike("PV01-E2-v1").unwrap();
        let spatial = SpatialReleaseManifest::e1(
            "e1-fixed",
            SpatialReference::prototype().unwrap(),
            Vec::new(),
        )
        .unwrap();
        let spatial_hash = sha256_hex(&canonical_json(&spatial).unwrap());
        let world = build_fixed(&spec, &pv01_fixed_input().unwrap()).unwrap();
        let tectonic =
            TectonicManifest::from_world("pv01-release", &world, "e1-fixed", spatial_hash.clone());
        let tectonic_hash = sha256_hex(&canonical_json(&tectonic).unwrap());
        let bundle = TerrainMaterialInputBundle::build(
            &spec,
            &spatial,
            spatial_hash,
            &tectonic,
            tectonic_hash,
        )
        .unwrap();

        for (profile, namespace, expected) in [
            (
                RunProfile::FixedTest,
                "test/pv01",
                AdmissionConclusion::Admitted,
            ),
            (
                RunProfile::Diagnostic,
                "diagnostic/pv01",
                AdmissionConclusion::DiagnosticOnly,
            ),
            (
                RunProfile::StandardGlobal,
                "release/global",
                AdmissionConclusion::Rejected,
            ),
            (
                RunProfile::RegionalRefinement,
                "release/regional",
                AdmissionConclusion::Rejected,
            ),
        ] {
            let plan = TerrainMaterialRunPlan::new(profile, namespace).unwrap();
            assert_eq!(admit(&bundle, &plan, &tectonic).conclusion, expected);
        }

        let mut value = serde_json::to_value(&bundle).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("test_oracle".into(), serde_json::json!({"C1":"CONVERGENT"}));
        assert!(serde_json::from_value::<TerrainMaterialInputBundle>(value).is_err());
    }
}
