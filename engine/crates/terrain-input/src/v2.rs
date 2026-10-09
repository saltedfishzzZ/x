//! E2.1 v2 M1权威请求、派生声明与完整准入重算。

use crate::{RunProfile, TerrainInputError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use world_contracts::{CapabilityState, canonical_json, sha256_hex};
use world_spec::DEFAULT_RADIUS_METRES;
use world_spec::v2::{GenerationSpecV2, ReleaseRefV2, WorldIntentSpecV2, WorldKindV2};

pub const M1_INPUT_SCHEMA_V2: &str = "m1-input-release-v2";
pub const TERRAIN_MODEL_V2: &str = "terrain-material-prototype-v2";
pub const PARAMETER_PROFILE_V1: &str = "earthlike-prototype-v1";
pub const RUN_POLICY_V2: &str = "terrain-run-policy-v2";
pub const WORLD_INTENT_FILE_V2: &str = "world-intent-v2.json";
pub const GENERATION_SPEC_FILE_V2: &str = "generation-spec-v2.json";
const M1_RELEASE_FILE_V2: &str = "m1-input-release-v2.json";
const M1_RELEASE_HASH_FILE_V2: &str = "m1-input-release-v2.sha256";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerticalDatumDescriptorV2 {
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
pub struct M1AuthoritativeRequestV2 {
    pub world_intent: ReleaseRefV2,
    pub generation_spec: ReleaseRefV2,
    pub spatial_release: ReleaseRefV2,
    pub tectonic_release: ReleaseRefV2,
    pub terrain_material_model: String,
    pub parameter_profile: String,
    pub vertical_datum: VerticalDatumDescriptorV2,
    pub required_capabilities: Vec<String>,
    pub profile: RunProfile,
    pub output_namespace: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdmissionConclusionV2 {
    Admitted,
    DiagnosticOnly,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionReasonV2 {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionClaimV2 {
    pub conclusion: AdmissionConclusionV2,
    pub reasons: Vec<AdmissionReasonV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M1DerivedClaimsV2 {
    pub input_bundle_id: String,
    pub run_plan_id: String,
    pub dependency_closure: Vec<ReleaseRefV2>,
    pub admission: AdmissionClaimV2,
    pub capabilities: BTreeMap<String, CapabilityState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M1InputReleaseV2 {
    pub schema: String,
    pub m1_release_id: String,
    pub request: M1AuthoritativeRequestV2,
    pub claims: M1DerivedClaimsV2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedM1InputReleaseV2 {
    pub path: PathBuf,
    pub release_sha256: String,
    pub visibility: release_store::VisibilityGuarantee,
    pub durability: release_store::DurabilityGuarantee,
    pub reused_existing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedSpatialReleaseV2 {
    pub release_ref: ReleaseRefV2,
    pub capabilities: BTreeMap<String, CapabilityState>,
    pub direct_dependencies: Vec<ReleaseRefV2>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedTectonicReleaseV2 {
    pub release_ref: ReleaseRefV2,
    pub world_id: String,
    pub generation_identity: String,
    pub spatial_release: ReleaseRefV2,
    pub capabilities: BTreeMap<String, CapabilityState>,
    pub direct_dependencies: Vec<ReleaseRefV2>,
    pub validation_passed: bool,
}

pub trait ReleaseResolverV2 {
    /// Resolves and decodes the exact world-intent artifact.
    ///
    /// # Errors
    /// Returns an input error when the exact identity or content is unavailable.
    fn world_intent(
        &self,
        reference: &ReleaseRefV2,
    ) -> Result<WorldIntentSpecV2, TerrainInputError>;

    /// Resolves and decodes the exact generation-spec artifact.
    ///
    /// # Errors
    /// Returns an input error when the exact identity or content is unavailable.
    fn generation_spec(
        &self,
        reference: &ReleaseRefV2,
    ) -> Result<GenerationSpecV2, TerrainInputError>;

    /// Resolves a previously validated v2 spatial release.
    ///
    /// # Errors
    /// Returns an input error when the exact release is unavailable or corrupt.
    fn spatial_release(
        &self,
        reference: &ReleaseRefV2,
    ) -> Result<ResolvedSpatialReleaseV2, TerrainInputError>;

    /// Resolves a previously validated v2 tectonic release.
    ///
    /// # Errors
    /// Returns an input error when the exact release is unavailable or corrupt.
    fn tectonic_release(
        &self,
        reference: &ReleaseRefV2,
    ) -> Result<ResolvedTectonicReleaseV2, TerrainInputError>;
}

#[derive(Clone, Debug)]
pub struct FileSystemReleaseResolverV2 {
    store_root: PathBuf,
}

impl FileSystemReleaseResolverV2 {
    #[must_use]
    pub fn new(store_root: impl Into<PathBuf>) -> Self {
        Self {
            store_root: store_root.into(),
        }
    }

    fn release_path(&self, reference: &ReleaseRefV2) -> Result<PathBuf, TerrainInputError> {
        validate_release_ref(reference)?;
        let id = release_store::SafeStoreId::parse(&reference.release_id)?;
        let path = self.store_root.join("releases").join(id.as_str());
        let root: release_store::ReleaseRoot =
            serde_json::from_slice(&std::fs::read(path.join("release.json"))?)?;
        if root.release_id != reference.release_id {
            return Err(TerrainInputError::Invalid(
                "release root identity differs from the requested reference".to_owned(),
            ));
        }
        Ok(path)
    }

    fn read_exact_json<T: serde::de::DeserializeOwned + Serialize>(
        &self,
        reference: &ReleaseRefV2,
        file_name: &str,
    ) -> Result<T, TerrainInputError> {
        let bytes = std::fs::read(self.release_path(reference)?.join(file_name))?;
        if sha256_hex(&bytes) != reference.manifest_sha256 {
            return Err(TerrainInputError::HashMismatch);
        }
        let value: T = serde_json::from_slice(&bytes)?;
        if canonical_json(&value)? != bytes {
            return Err(TerrainInputError::NonCanonical);
        }
        Ok(value)
    }
}

impl ReleaseResolverV2 for FileSystemReleaseResolverV2 {
    fn world_intent(
        &self,
        reference: &ReleaseRefV2,
    ) -> Result<WorldIntentSpecV2, TerrainInputError> {
        let intent: WorldIntentSpecV2 = self.read_exact_json(reference, WORLD_INTENT_FILE_V2)?;
        intent.validate()?;
        Ok(intent)
    }

    fn generation_spec(
        &self,
        reference: &ReleaseRefV2,
    ) -> Result<GenerationSpecV2, TerrainInputError> {
        self.read_exact_json(reference, GENERATION_SPEC_FILE_V2)
    }

    fn spatial_release(
        &self,
        reference: &ReleaseRefV2,
    ) -> Result<ResolvedSpatialReleaseV2, TerrainInputError> {
        let path = self.release_path(reference)?;
        let manifest = sphere_release::v2::open_v2(&path)
            .map_err(|error| TerrainInputError::Invalid(error.to_string()))?;
        let bytes = canonical_json(&manifest)?;
        if manifest.spatial_release_id != reference.release_id
            || sha256_hex(&bytes) != reference.manifest_sha256
        {
            return Err(TerrainInputError::HashMismatch);
        }
        Ok(ResolvedSpatialReleaseV2 {
            release_ref: reference.clone(),
            capabilities: manifest.capabilities,
            direct_dependencies: manifest.direct_dependencies,
        })
    }

    fn tectonic_release(
        &self,
        reference: &ReleaseRefV2,
    ) -> Result<ResolvedTectonicReleaseV2, TerrainInputError> {
        let path = self.release_path(reference)?;
        let manifest = tectonics_release::v2::open_v2(&path)
            .map_err(|error| TerrainInputError::Invalid(error.to_string()))?;
        let bytes = canonical_json(&manifest)?;
        if manifest.tectonic_release_id != reference.release_id
            || sha256_hex(&bytes) != reference.manifest_sha256
        {
            return Err(TerrainInputError::HashMismatch);
        }
        Ok(ResolvedTectonicReleaseV2 {
            release_ref: reference.clone(),
            world_id: manifest.world_id,
            generation_identity: manifest.generation_identity,
            spatial_release: manifest.spatial_release,
            capabilities: manifest.capabilities,
            direct_dependencies: manifest.direct_dependencies,
            validation_passed: manifest.validation_passed,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationLevelV2 {
    Structural,
    SelfConsistency,
    FullAdmission,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationStatusV2 {
    Valid,
    Invalid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationFindingV2 {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct M1ValidationReportV2 {
    pub validation_level: ValidationLevelV2,
    pub status: ValidationStatusV2,
    pub release_id: String,
    pub release_sha256: String,
    pub findings: Vec<ValidationFindingV2>,
    pub recomputed_claims: Option<M1DerivedClaimsV2>,
}

impl M1AuthoritativeRequestV2 {
    /// Creates a normalized request whose scientific inputs are exact references.
    ///
    /// # Errors
    /// Rejects path-like namespaces, malformed references, or noncanonical capabilities.
    pub fn new(
        world_intent: ReleaseRefV2,
        generation_spec: ReleaseRefV2,
        spatial_release: ReleaseRefV2,
        tectonic_release: ReleaseRefV2,
        profile: RunProfile,
        output_namespace: impl Into<String>,
        mut required_capabilities: Vec<String>,
    ) -> Result<Self, TerrainInputError> {
        required_capabilities.sort();
        if required_capabilities
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(TerrainInputError::Invalid(
                "required capabilities contain duplicates".into(),
            ));
        }
        let request = Self {
            world_intent,
            generation_spec,
            spatial_release,
            tectonic_release,
            terrain_material_model: TERRAIN_MODEL_V2.to_owned(),
            parameter_profile: PARAMETER_PROFILE_V1.to_owned(),
            vertical_datum: VerticalDatumDescriptorV2 {
                datum_id: "reference-sphere-v2".to_owned(),
                origin: "planet-centre".to_owned(),
                radial_direction_source: "E1 authoritative UnitVec3".to_owned(),
                reference_radius_metres: DEFAULT_RADIUS_METRES,
                elevation_unit: "m".to_owned(),
                zero_surface: "reference-sphere".to_owned(),
                sea_level_relation: "SEPARATE_NOT_AVAILABLE".to_owned(),
            },
            required_capabilities,
            profile,
            output_namespace: output_namespace.into(),
        };
        request.validate_structure()?;
        Ok(request)
    }

    fn validate_structure(&self) -> Result<(), TerrainInputError> {
        for reference in [
            &self.world_intent,
            &self.generation_spec,
            &self.spatial_release,
            &self.tectonic_release,
        ] {
            validate_release_ref(reference)?;
        }
        if self.terrain_material_model != TERRAIN_MODEL_V2
            || self.parameter_profile != PARAMETER_PROFILE_V1
            || self.vertical_datum.reference_radius_metres != DEFAULT_RADIUS_METRES
            || self.output_namespace.is_empty()
            || self.output_namespace.starts_with('/')
            || self.output_namespace.contains("..")
            || self
                .required_capabilities
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .required_capabilities
                .iter()
                .any(|value| !safe_id(value))
        {
            return Err(TerrainInputError::Invalid(
                "M1 v2 authoritative request is noncanonical".into(),
            ));
        }
        Ok(())
    }
}

impl M1InputReleaseV2 {
    /// Builds an M1 release using claims derived from actual resolved dependencies.
    ///
    /// # Errors
    /// Rejects missing, inconsistent, or noncanonical upstream inputs.
    pub fn build(
        request: M1AuthoritativeRequestV2,
        resolver: &impl ReleaseResolverV2,
    ) -> Result<Self, TerrainInputError> {
        request.validate_structure()?;
        let claims = derive_claims(&request, resolver).map_err(|finding| {
            TerrainInputError::Invalid(format!("{}: {}", finding.code, finding.detail))
        })?;
        let mut release = Self {
            schema: M1_INPUT_SCHEMA_V2.to_owned(),
            m1_release_id: String::new(),
            request,
            claims,
        };
        release.m1_release_id = compute_release_id(&release)?;
        Ok(release)
    }

    /// Checks schema and authoritative request shape without trusting or resolving claims.
    #[must_use]
    pub fn validate_structure(&self) -> M1ValidationReportV2 {
        let mut findings = Vec::new();
        if self.schema != M1_INPUT_SCHEMA_V2 {
            findings.push(finding("UNSUPPORTED_SCHEMA", "M1 schema is not v2"));
        }
        if let Err(error) = self.request.validate_structure() {
            findings.push(finding("INVALID_REQUEST", &error.to_string()));
        }
        validation_report(self, ValidationLevelV2::Structural, findings, None)
    }

    /// Checks canonical release identity in addition to structural validity.
    #[must_use]
    pub fn validate_self_consistency(&self) -> M1ValidationReportV2 {
        let mut report = self.validate_structure();
        report.validation_level = ValidationLevelV2::SelfConsistency;
        match compute_release_id(self) {
            Ok(expected) if expected != self.m1_release_id => report.findings.push(finding(
                "M1_RELEASE_ID_MISMATCH",
                "stored M1 release ID differs from canonical content",
            )),
            Err(error) => report
                .findings
                .push(finding("M1_RELEASE_ID_INVALID", &error.to_string())),
            _ => {}
        }
        report.status = if report.findings.is_empty() {
            ValidationStatusV2::Valid
        } else {
            ValidationStatusV2::Invalid
        };
        report
    }

    /// Recomputes every derived claim from resolved upstream data.
    #[must_use]
    pub fn validate_full_admission(
        &self,
        resolver: &impl ReleaseResolverV2,
    ) -> M1ValidationReportV2 {
        let mut findings = Vec::new();
        if self.schema != M1_INPUT_SCHEMA_V2 {
            findings.push(finding("UNSUPPORTED_SCHEMA", "M1 schema is not v2"));
        }
        if let Err(error) = self.request.validate_structure() {
            findings.push(finding("INVALID_REQUEST", &error.to_string()));
        }
        let recomputed = match derive_claims(&self.request, resolver) {
            Ok(claims) => Some(claims),
            Err(error) => {
                findings.push(error);
                None
            }
        };
        if let Some(claims) = &recomputed {
            if self.claims.input_bundle_id != claims.input_bundle_id {
                findings.push(finding(
                    "INPUT_BUNDLE_ID_MISMATCH",
                    "stored InputBundleId differs from resolved inputs",
                ));
            }
            if self.claims.run_plan_id != claims.run_plan_id {
                findings.push(finding(
                    "RUN_PLAN_ID_MISMATCH",
                    "stored RunPlanId differs from the execution request",
                ));
            }
            if self.claims.dependency_closure != claims.dependency_closure {
                findings.push(finding(
                    "DEPENDENCY_CLOSURE_MISMATCH",
                    "stored dependency closure differs from resolved upstreams",
                ));
            }
            if self.claims.admission != claims.admission {
                findings.push(finding(
                    "ADMISSION_CLAIM_MISMATCH",
                    "stored admission differs from the admission policy",
                ));
            }
            if self.claims.capabilities != claims.capabilities {
                findings.push(finding(
                    "CAPABILITY_CLAIMS_MISMATCH",
                    "stored capabilities differ from resolved data",
                ));
            }
        }
        match compute_release_id(self) {
            Ok(expected) if expected != self.m1_release_id => findings.push(finding(
                "M1_RELEASE_ID_MISMATCH",
                "stored M1 release ID differs from canonical content",
            )),
            Err(error) => findings.push(finding("M1_RELEASE_ID_INVALID", &error.to_string())),
            _ => {}
        }
        validation_report(self, ValidationLevelV2::FullAdmission, findings, recomputed)
    }
}

/// Publishes an M1 v2 release only after full upstream admission reconstruction.
///
/// # Errors
/// Rejects invalid claims, unavailable upstreams, unsafe IDs and storage failures.
pub fn publish_v2(
    store_root: &std::path::Path,
    candidate_id: &str,
    release: &M1InputReleaseV2,
    resolver: &impl ReleaseResolverV2,
) -> Result<PublishedM1InputReleaseV2, TerrainInputError> {
    let report = release.validate_full_admission(resolver);
    if report.status != ValidationStatusV2::Valid {
        return Err(TerrainInputError::Invalid(format!(
            "M1 full admission failed: {:?}",
            report.findings
        )));
    }
    let store = release_store::ReleaseStore::new(store_root)?;
    let candidate = store.create_candidate_v2(candidate_id, &release.m1_release_id)?;
    let bytes = canonical_json(release)?;
    let hash = sha256_hex(&bytes);
    std::fs::write(candidate.join(M1_RELEASE_FILE_V2), bytes)?;
    std::fs::write(candidate.join(M1_RELEASE_HASH_FILE_V2), hash.as_bytes())?;
    store.mark_candidate_validated(&candidate)?;
    let committed = store.commit_v2(
        &candidate,
        &release_store::ReleaseRoot {
            release_id: release.m1_release_id.clone(),
            asset_ids: Vec::new(),
        },
    )?;
    Ok(PublishedM1InputReleaseV2 {
        path: committed.path,
        release_sha256: hash,
        visibility: committed.visibility,
        durability: committed.durability,
        reused_existing: committed.reused_existing,
    })
}

/// Opens an M1 v2 release and defaults to full upstream admission validation.
///
/// # Errors
/// Rejects byte, canonical, release-root, claim or upstream inconsistencies.
pub fn open_v2(
    path: &std::path::Path,
    resolver: &impl ReleaseResolverV2,
) -> Result<M1InputReleaseV2, TerrainInputError> {
    let bytes = std::fs::read(path.join(M1_RELEASE_FILE_V2))?;
    let expected_hash = std::fs::read_to_string(path.join(M1_RELEASE_HASH_FILE_V2))?;
    if sha256_hex(&bytes) != expected_hash {
        return Err(TerrainInputError::HashMismatch);
    }
    let release: M1InputReleaseV2 = serde_json::from_slice(&bytes)?;
    if canonical_json(&release)? != bytes {
        return Err(TerrainInputError::NonCanonical);
    }
    let root: release_store::ReleaseRoot =
        serde_json::from_slice(&std::fs::read(path.join("release.json"))?)?;
    if root.release_id != release.m1_release_id || !root.asset_ids.is_empty() {
        return Err(TerrainInputError::Invalid(
            "M1 v2 release root mismatch".to_owned(),
        ));
    }
    let report = release.validate_full_admission(resolver);
    if report.status != ValidationStatusV2::Valid {
        return Err(TerrainInputError::Invalid(format!(
            "M1 full admission failed: {:?}",
            report.findings
        )));
    }
    Ok(release)
}

fn validation_report(
    release: &M1InputReleaseV2,
    level: ValidationLevelV2,
    findings: Vec<ValidationFindingV2>,
    recomputed_claims: Option<M1DerivedClaimsV2>,
) -> M1ValidationReportV2 {
    let release_sha256 = canonical_json(release)
        .map(|bytes| sha256_hex(&bytes))
        .unwrap_or_default();
    M1ValidationReportV2 {
        validation_level: level,
        status: if findings.is_empty() {
            ValidationStatusV2::Valid
        } else {
            ValidationStatusV2::Invalid
        },
        release_id: release.m1_release_id.clone(),
        release_sha256,
        findings,
        recomputed_claims,
    }
}

fn derive_claims(
    request: &M1AuthoritativeRequestV2,
    resolver: &impl ReleaseResolverV2,
) -> Result<M1DerivedClaimsV2, ValidationFindingV2> {
    request
        .validate_structure()
        .map_err(|error| finding("INVALID_REQUEST", &error.to_string()))?;
    let upstreams = resolve_upstreams(request, resolver)?;
    let bundle_bytes = canonical_json(&(
        "input-bundle-v2",
        &upstreams.generation_identity,
        &upstreams.dependency_closure,
        &request.terrain_material_model,
        &request.parameter_profile,
        &request.vertical_datum,
        &request.required_capabilities,
    ))
    .map_err(|error| finding("INPUT_BUNDLE_INVALID", &error.to_string()))?;
    let input_bundle_id = format!("input-v2-{}", &sha256_hex(&bundle_bytes)[..24]);
    let run_bytes = canonical_json(&(
        "run-plan-v2",
        &input_bundle_id,
        request.profile,
        &request.output_namespace,
        RUN_POLICY_V2,
    ))
    .map_err(|error| finding("RUN_PLAN_INVALID", &error.to_string()))?;
    let run_plan_id = format!("run-plan-v2-{}", &sha256_hex(&run_bytes)[..24]);
    let admission = admission(&upstreams.intent, request);
    let capabilities = capability_claims(&upstreams.intent, &upstreams.tectonic_capabilities);
    Ok(M1DerivedClaimsV2 {
        input_bundle_id,
        run_plan_id,
        dependency_closure: upstreams.dependency_closure,
        admission,
        capabilities,
    })
}

struct ResolvedUpstreamsV2 {
    intent: WorldIntentSpecV2,
    generation_identity: String,
    dependency_closure: Vec<ReleaseRefV2>,
    tectonic_capabilities: BTreeMap<String, CapabilityState>,
}

fn resolve_upstreams(
    request: &M1AuthoritativeRequestV2,
    resolver: &impl ReleaseResolverV2,
) -> Result<ResolvedUpstreamsV2, ValidationFindingV2> {
    let intent = resolver
        .world_intent(&request.world_intent)
        .map_err(|error| finding("WORLD_INTENT_UNAVAILABLE", &error.to_string()))?;
    intent
        .validate()
        .map_err(|error| finding("WORLD_INTENT_INVALID", &error.to_string()))?;
    let intent_bytes = canonical_json(&intent)
        .map_err(|error| finding("WORLD_INTENT_INVALID", &error.to_string()))?;
    let intent_identity = intent
        .identity()
        .map_err(|error| finding("WORLD_INTENT_INVALID", &error.to_string()))?;
    if request.world_intent.release_id != intent_identity.world_id
        || request.world_intent.manifest_sha256 != sha256_hex(&intent_bytes)
    {
        return Err(finding(
            "DEPENDENCY_HASH_MISMATCH",
            "world-intent identity or content hash does not match its reference",
        ));
    }

    let generation_spec = resolver
        .generation_spec(&request.generation_spec)
        .map_err(|error| finding("GENERATION_SPEC_UNAVAILABLE", &error.to_string()))?;
    let generation_identity = generation_spec
        .identity(&intent)
        .map_err(|error| finding("GENERATION_IDENTITY_MISMATCH", &error.to_string()))?;
    let generation_bytes = canonical_json(&generation_spec)
        .map_err(|error| finding("GENERATION_SPEC_INVALID", &error.to_string()))?;
    if request.generation_spec.release_id != generation_identity.generation_identity
        || request.generation_spec.manifest_sha256 != sha256_hex(&generation_bytes)
        || generation_spec.spatial_release != request.spatial_release
    {
        return Err(finding(
            "GENERATION_IDENTITY_MISMATCH",
            "generation specification does not match its identity or spatial release",
        ));
    }

    let spatial = resolver
        .spatial_release(&request.spatial_release)
        .map_err(|error| finding("SPATIAL_RELEASE_UNAVAILABLE", &error.to_string()))?;
    if spatial.release_ref != request.spatial_release {
        return Err(finding(
            "DEPENDENCY_HASH_MISMATCH",
            "resolved spatial release differs from its exact reference",
        ));
    }
    let tectonic = resolver
        .tectonic_release(&request.tectonic_release)
        .map_err(|error| finding("TECTONIC_RELEASE_UNAVAILABLE", &error.to_string()))?;
    if tectonic.release_ref != request.tectonic_release
        || tectonic.world_id != intent_identity.world_id
        || tectonic.generation_identity != generation_identity.generation_identity
        || tectonic.spatial_release != request.spatial_release
    {
        return Err(finding(
            "GENERATION_IDENTITY_MISMATCH",
            "resolved tectonic release belongs to a different generation",
        ));
    }
    if !tectonic.validation_passed {
        return Err(finding(
            "TECTONIC_EVIDENCE_INCOMPLETE",
            "tectonic release did not pass its own semantic validation",
        ));
    }
    for capability in &request.required_capabilities {
        if tectonic.capabilities.get(capability) != Some(&CapabilityState::Available) {
            return Err(finding(
                "MISSING_CAPABILITY",
                &format!("required tectonic capability is unavailable: {capability}"),
            ));
        }
    }

    let mut dependency_closure = vec![
        request.world_intent.clone(),
        request.generation_spec.clone(),
        request.spatial_release.clone(),
        request.tectonic_release.clone(),
    ];
    dependency_closure.extend(spatial.direct_dependencies);
    dependency_closure.extend(tectonic.direct_dependencies);
    dependency_closure.sort_by(|left, right| {
        (&left.release_id, &left.manifest_sha256).cmp(&(&right.release_id, &right.manifest_sha256))
    });
    dependency_closure.dedup();
    if dependency_closure
        .iter()
        .any(|reference| validate_release_ref(reference).is_err())
    {
        return Err(finding(
            "INCOMPLETE_DEPENDENCY_CLOSURE",
            "dependency closure contains an invalid exact reference",
        ));
    }

    Ok(ResolvedUpstreamsV2 {
        intent,
        generation_identity: generation_identity.generation_identity,
        dependency_closure,
        tectonic_capabilities: tectonic.capabilities,
    })
}

fn admission(intent: &WorldIntentSpecV2, request: &M1AuthoritativeRequestV2) -> AdmissionClaimV2 {
    let (conclusion, code, detail) = match request.profile {
        RunProfile::FixedTest
            if intent.world_kind == WorldKindV2::FixedTest
                && request.output_namespace.starts_with("test/") =>
        {
            (
                AdmissionConclusionV2::Admitted,
                "FIXED_TEST_ADMITTED",
                "固定测试输入获准进入隔离测试命名空间",
            )
        }
        RunProfile::FixedTest => (
            AdmissionConclusionV2::Rejected,
            "FIXED_TEST_ISOLATION_FAILED",
            "固定测试要求固定世界身份和test/命名空间",
        ),
        RunProfile::Diagnostic => (
            AdmissionConclusionV2::DiagnosticOnly,
            "PROTOTYPE_DIAGNOSTIC_ONLY",
            "E2.1普通构造仍只允许诊断",
        ),
        RunProfile::StandardGlobal | RunProfile::RegionalRefinement => (
            AdmissionConclusionV2::Rejected,
            "RUN_PROFILE_NOT_AVAILABLE",
            "正式地形运行等待后续层能力",
        ),
    };
    AdmissionClaimV2 {
        conclusion,
        reasons: vec![AdmissionReasonV2 {
            code: code.to_owned(),
            detail: detail.to_owned(),
        }],
    }
}

fn capability_claims(
    intent: &WorldIntentSpecV2,
    tectonic: &BTreeMap<String, CapabilityState>,
) -> BTreeMap<String, CapabilityState> {
    let mut claims = BTreeMap::new();
    claims.insert("diagnostic".to_owned(), CapabilityState::Available);
    claims.insert(
        "fixed_test".to_owned(),
        if intent.world_kind == WorldKindV2::FixedTest {
            CapabilityState::Available
        } else {
            CapabilityState::NotAvailable
        },
    );
    claims.insert("standard_global".to_owned(), CapabilityState::NotAvailable);
    claims.insert(
        "regional_refinement".to_owned(),
        CapabilityState::NotAvailable,
    );
    for (name, state) in tectonic {
        claims.insert(format!("tectonics/{name}"), *state);
    }
    claims
}

fn compute_release_id(release: &M1InputReleaseV2) -> Result<String, TerrainInputError> {
    let bytes = canonical_json(&(M1_INPUT_SCHEMA_V2, &release.request, &release.claims))?;
    Ok(format!("m1-v2-{}", &sha256_hex(&bytes)[..24]))
}

fn validate_release_ref(reference: &ReleaseRefV2) -> Result<(), TerrainInputError> {
    if release_store::SafeStoreId::parse(&reference.release_id).is_err()
        || !sha256_digest(&reference.manifest_sha256)
    {
        return Err(TerrainInputError::Invalid(
            "release reference contains an invalid identity or SHA-256".into(),
        ));
    }
    Ok(())
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
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

fn finding(code: &str, detail: &str) -> ValidationFindingV2 {
    ValidationFindingV2 {
        code: code.to_owned(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::cell::RefCell;
    use std::collections::BTreeSet;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MemoryResolver {
        intent: RefCell<WorldIntentSpecV2>,
        generation: GenerationSpecV2,
        spatial: ResolvedSpatialReleaseV2,
        tectonic: ResolvedTectonicReleaseV2,
    }

    #[derive(Deserialize)]
    struct ForgedClaimsFixture {
        fixture_id: String,
        schema: String,
        cases: Vec<ForgedClaimCase>,
    }

    #[derive(Deserialize)]
    struct ForgedClaimCase {
        mutation: String,
        expected_code: String,
    }

    impl ReleaseResolverV2 for MemoryResolver {
        fn world_intent(
            &self,
            _reference: &ReleaseRefV2,
        ) -> Result<WorldIntentSpecV2, TerrainInputError> {
            Ok(self.intent.borrow().clone())
        }

        fn generation_spec(
            &self,
            _reference: &ReleaseRefV2,
        ) -> Result<GenerationSpecV2, TerrainInputError> {
            Ok(self.generation.clone())
        }

        fn spatial_release(
            &self,
            _reference: &ReleaseRefV2,
        ) -> Result<ResolvedSpatialReleaseV2, TerrainInputError> {
            Ok(self.spatial.clone())
        }

        fn tectonic_release(
            &self,
            _reference: &ReleaseRefV2,
        ) -> Result<ResolvedTectonicReleaseV2, TerrainInputError> {
            Ok(self.tectonic.clone())
        }
    }

    fn setup() -> (M1InputReleaseV2, MemoryResolver) {
        let intent = WorldIntentSpecV2::earthlike_natural("m1-v2-test").unwrap();
        let intent_identity = intent.identity().unwrap();
        let intent_ref = ReleaseRefV2 {
            release_id: intent_identity.world_id.clone(),
            manifest_sha256: sha256_hex(&canonical_json(&intent).unwrap()),
        };
        let spatial_ref = ReleaseRefV2 {
            release_id: "spatial-v2".to_owned(),
            manifest_sha256: "a".repeat(64),
        };
        let generation = GenerationSpecV2::new(
            &intent,
            spatial_ref.clone(),
            BTreeMap::from([("tectonics".to_owned(), "tectonics-v2".to_owned())]),
        )
        .unwrap();
        let generation_identity = generation.identity(&intent).unwrap();
        let generation_ref = ReleaseRefV2 {
            release_id: generation_identity.generation_identity.clone(),
            manifest_sha256: sha256_hex(&canonical_json(&generation).unwrap()),
        };
        let tectonic_ref = ReleaseRefV2 {
            release_id: "tectonics-v2".to_owned(),
            manifest_sha256: "b".repeat(64),
        };
        let capabilities = BTreeMap::from([
            ("main_plate_domains".to_owned(), CapabilityState::Available),
            ("plate_euler_motion".to_owned(), CapabilityState::Available),
        ]);
        let resolver = MemoryResolver {
            intent: RefCell::new(intent),
            generation,
            spatial: ResolvedSpatialReleaseV2 {
                release_ref: spatial_ref.clone(),
                capabilities: BTreeMap::new(),
                direct_dependencies: Vec::new(),
            },
            tectonic: ResolvedTectonicReleaseV2 {
                release_ref: tectonic_ref.clone(),
                world_id: intent_identity.world_id,
                generation_identity: generation_identity.generation_identity,
                spatial_release: spatial_ref.clone(),
                capabilities,
                direct_dependencies: vec![spatial_ref.clone()],
                validation_passed: true,
            },
        };
        let request = M1AuthoritativeRequestV2::new(
            intent_ref,
            generation_ref,
            spatial_ref,
            tectonic_ref,
            RunProfile::Diagnostic,
            "diagnostic/default",
            vec![
                "main_plate_domains".to_owned(),
                "plate_euler_motion".to_owned(),
            ],
        )
        .unwrap();
        let release = M1InputReleaseV2::build(request, &resolver).unwrap();
        (release, resolver)
    }

    fn codes(report: &M1ValidationReportV2) -> BTreeSet<&str> {
        report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect()
    }

    fn temp_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("m1-fs-{name}-{}-{nonce}", std::process::id()))
    }

    fn publish_json<T: Serialize>(
        store: &release_store::ReleaseStore,
        candidate_id: &str,
        release_id: &str,
        file_name: &str,
        value: &T,
    ) -> ReleaseRefV2 {
        let candidate = store.create_candidate_v2(candidate_id, release_id).unwrap();
        let bytes = canonical_json(value).unwrap();
        let hash = sha256_hex(&bytes);
        std::fs::write(candidate.join(file_name), bytes).unwrap();
        store.mark_candidate_validated(&candidate).unwrap();
        store
            .commit_v2(
                &candidate,
                &release_store::ReleaseRoot {
                    release_id: release_id.to_owned(),
                    asset_ids: Vec::new(),
                },
            )
            .unwrap();
        ReleaseRefV2 {
            release_id: release_id.to_owned(),
            manifest_sha256: hash,
        }
    }

    #[test]
    fn filesystem_resolver_closes_full_m1_admission_from_published_bytes() {
        use sphere_core::SpatialReference;
        use tectonics_core::v2::{build_kinematics, build_natural_skeleton};

        let root = temp_path("full");
        let store = release_store::ReleaseStore::new(&root).unwrap();
        let intent = WorldIntentSpecV2::earthlike_natural("m1-filesystem-v2").unwrap();
        let intent_identity = intent.identity().unwrap();
        let intent_ref = publish_json(
            &store,
            "intent-candidate",
            &intent_identity.world_id,
            WORLD_INTENT_FILE_V2,
            &intent,
        );

        let spatial_manifest = sphere_release::v2::SpatialReleaseManifestV2::build(
            "spatial-fs-v2",
            &SpatialReference::prototype().unwrap(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let spatial_published =
            sphere_release::v2::publish_v2(&root, "spatial-candidate", &spatial_manifest).unwrap();
        let spatial_ref = ReleaseRefV2 {
            release_id: spatial_manifest.spatial_release_id.clone(),
            manifest_sha256: spatial_published.manifest_sha256,
        };

        let generation = GenerationSpecV2::new(
            &intent,
            spatial_ref.clone(),
            BTreeMap::from([("tectonics".to_owned(), "tectonics-v2".to_owned())]),
        )
        .unwrap();
        let generation_identity = generation.identity(&intent).unwrap();
        let generation_ref = publish_json(
            &store,
            "generation-candidate",
            &generation_identity.generation_identity,
            GENERATION_SPEC_FILE_V2,
            &generation,
        );

        let skeleton = build_natural_skeleton(&intent).unwrap();
        let kinematics = build_kinematics(&intent, &generation_identity, &skeleton).unwrap();
        let tectonic_manifest = tectonics_release::v2::TectonicManifestV2::build(
            "tectonic-fs-v2",
            &generation_identity,
            spatial_ref.clone(),
            skeleton,
            kinematics,
        )
        .unwrap();
        let tectonic_published =
            tectonics_release::v2::publish_v2(&root, "tectonic-candidate", &tectonic_manifest)
                .unwrap();
        let tectonic_ref = ReleaseRefV2 {
            release_id: tectonic_manifest.tectonic_release_id.clone(),
            manifest_sha256: tectonic_published.manifest_sha256,
        };

        let resolver = FileSystemReleaseResolverV2::new(&root);
        let request = M1AuthoritativeRequestV2::new(
            intent_ref,
            generation_ref,
            spatial_ref,
            tectonic_ref,
            RunProfile::Diagnostic,
            "diagnostic/filesystem",
            vec![
                "main_plate_domains".to_owned(),
                "plate_euler_motion".to_owned(),
            ],
        )
        .unwrap();
        let release = M1InputReleaseV2::build(request, &resolver).unwrap();
        assert_eq!(
            release.validate_full_admission(&resolver).status,
            ValidationStatusV2::Valid
        );
        let published = publish_v2(&root, "m1-candidate", &release, &resolver).unwrap();
        assert_eq!(open_v2(&published.path, &resolver).unwrap(), release);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_admission_recomputes_a_valid_diagnostic_release() {
        let (release, resolver) = setup();
        let report = release.validate_full_admission(&resolver);
        assert_eq!(report.status, ValidationStatusV2::Valid);
        assert_eq!(
            release.claims.admission.conclusion,
            AdmissionConclusionV2::DiagnosticOnly
        );
        assert!(report.findings.is_empty());
    }

    #[test]
    fn edited_admission_is_rejected_even_after_release_id_is_recomputed() {
        let (mut release, resolver) = setup();
        release.claims.admission = AdmissionClaimV2 {
            conclusion: AdmissionConclusionV2::Admitted,
            reasons: vec![AdmissionReasonV2 {
                code: "FORGED".to_owned(),
                detail: "forged admission".to_owned(),
            }],
        };
        release.m1_release_id = compute_release_id(&release).unwrap();
        assert_eq!(
            release.validate_self_consistency().status,
            ValidationStatusV2::Valid
        );
        let report = release.validate_full_admission(&resolver);
        assert_eq!(report.status, ValidationStatusV2::Invalid);
        assert!(codes(&report).contains("ADMISSION_CLAIM_MISMATCH"));
    }

    #[test]
    fn forged_claim_fixture_is_executable() {
        let fixture: ForgedClaimsFixture = serde_json::from_slice(include_bytes!(
            "../../../fixtures/e2.1/m1/E21M01-forged-claims.json"
        ))
        .unwrap();
        assert_eq!(fixture.fixture_id, "E21-M01");
        assert_eq!(fixture.schema, "e2.1-m1-forged-claims-fixture-v1");
        for case in fixture.cases {
            let (mut release, resolver) = setup();
            match case.mutation.as_str() {
                "admission" => {
                    release.claims.admission = AdmissionClaimV2 {
                        conclusion: AdmissionConclusionV2::Admitted,
                        reasons: vec![AdmissionReasonV2 {
                            code: "FORGED".to_owned(),
                            detail: "fixture mutation".to_owned(),
                        }],
                    };
                }
                "dependency_closure" => {
                    release.claims.dependency_closure.pop();
                }
                "capabilities" => {
                    release
                        .claims
                        .capabilities
                        .insert("standard_global".to_owned(), CapabilityState::Available);
                }
                "input_bundle_id" => {
                    release.claims.input_bundle_id = "input-v2-forged".to_owned();
                }
                "run_plan_id" => {
                    release.claims.run_plan_id = "run-plan-v2-forged".to_owned();
                }
                "upstream_content" => {
                    resolver.intent.borrow_mut().normalized_seed = "replacement".to_owned();
                }
                mutation => panic!("unknown fixture mutation: {mutation}"),
            }
            release.m1_release_id = compute_release_id(&release).unwrap();
            let report = release.validate_full_admission(&resolver);
            assert_eq!(report.status, ValidationStatusV2::Invalid);
            assert!(
                codes(&report).contains(case.expected_code.as_str()),
                "{} did not produce {}: {:?}",
                case.mutation,
                case.expected_code,
                report.findings
            );
        }
    }

    #[test]
    fn edited_dependency_closure_is_rejected_after_resigning() {
        let (mut release, resolver) = setup();
        release.claims.dependency_closure.pop();
        release.m1_release_id = compute_release_id(&release).unwrap();
        let report = release.validate_full_admission(&resolver);
        assert_eq!(report.status, ValidationStatusV2::Invalid);
        assert!(codes(&report).contains("DEPENDENCY_CLOSURE_MISMATCH"));
    }

    #[test]
    fn edited_capability_bundle_and_run_ids_are_all_rejected() {
        let (mut release, resolver) = setup();
        release
            .claims
            .capabilities
            .insert("standard_global".to_owned(), CapabilityState::Available);
        release.claims.input_bundle_id = "input-v2-forged".to_owned();
        release.claims.run_plan_id = "run-plan-v2-forged".to_owned();
        release.m1_release_id = compute_release_id(&release).unwrap();
        let report = release.validate_full_admission(&resolver);
        let result_codes = codes(&report);
        assert!(result_codes.contains("CAPABILITY_CLAIMS_MISMATCH"));
        assert!(result_codes.contains("INPUT_BUNDLE_ID_MISMATCH"));
        assert!(result_codes.contains("RUN_PLAN_ID_MISMATCH"));
    }

    #[test]
    fn upstream_content_replacement_with_the_same_reference_is_rejected() {
        let (release, resolver) = setup();
        resolver.intent.borrow_mut().normalized_seed = "replacement".to_owned();
        let report = release.validate_full_admission(&resolver);
        assert_eq!(report.status, ValidationStatusV2::Invalid);
        assert!(codes(&report).contains("DEPENDENCY_HASH_MISMATCH"));
    }

    #[test]
    fn a_generation_from_another_world_is_rejected() {
        let (release, mut resolver) = setup();
        let other = WorldIntentSpecV2::earthlike_natural("other-world").unwrap();
        resolver.generation = GenerationSpecV2::new(
            &other,
            release.request.spatial_release.clone(),
            BTreeMap::from([("tectonics".to_owned(), "tectonics-v2".to_owned())]),
        )
        .unwrap();
        let report = release.validate_full_admission(&resolver);
        assert_eq!(report.status, ValidationStatusV2::Invalid);
        assert!(codes(&report).contains("GENERATION_IDENTITY_MISMATCH"));
    }

    #[test]
    fn legacy_m1_json_cannot_decode_as_v2() {
        let legacy = serde_json::json!({
            "schema": "terrain-input-v1",
            "m1_release_id": "legacy",
            "input_bundle": {},
            "run_plan": {},
            "admission_report": {},
            "capabilities": {}
        });
        assert!(serde_json::from_value::<M1InputReleaseV2>(legacy).is_err());
    }
}
