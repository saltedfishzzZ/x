use serde::{Deserialize, Serialize};
use sphere_core::{
    CardinalDirection, CellId, EdgeOrientation, PatchPlan, SPATIAL_FRAME_ID, SpatialReference,
};
use std::collections::BTreeMap;
use world_contracts::CapabilityState;

pub const SPATIAL_MANIFEST_SCHEMA: &str = "E1-SPATIAL-RELEASE-v1";
pub const IDENTITY_FORMAT_VERSION: &str = "cs1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopologyEvidence {
    pub source_cell: CellId,
    pub source_edge: CardinalDirection,
    pub target_cell: CellId,
    pub target_edge: CardinalDirection,
    pub edge_orientation: EdgeOrientation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationEvidence {
    pub profile: String,
    pub l8_relative_area_tolerance: String,
    pub l0_topology: Vec<TopologyEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialReleaseManifest {
    pub schema: String,
    pub spatial_release_id: String,
    pub spatial_reference: SpatialReference,
    pub identity_format_version: String,
    pub face_order: [String; 6],
    pub boundary_tie_break: String,
    pub child_order: [String; 4],
    pub global_coverage_level: u8,
    pub patches: Vec<PatchPlan>,
    pub capabilities: BTreeMap<String, CapabilityState>,
    pub validation_evidence: ValidationEvidence,
    pub direct_dependency_hashes: BTreeMap<String, String>,
}

impl SpatialReleaseManifest {
    /// Builds the deterministic E1 spatial manifest from reconstructable facts.
    ///
    /// # Errors
    /// Propagates geometry or topology errors while producing validation evidence.
    pub fn e1(
        spatial_release_id: impl Into<String>,
        spatial_reference: SpatialReference,
        mut patches: Vec<PatchPlan>,
    ) -> Result<Self, sphere_core::SphereError> {
        patches.sort_by(|left, right| left.patch_id.cmp(&right.patch_id));
        let mut capabilities = BTreeMap::new();
        for name in [
            "cell_identity",
            "cell_geometry",
            "fixed_level_cap_cover",
            "neighbour_topology",
            "patch_2_to_1",
        ] {
            capabilities.insert(name.to_owned(), CapabilityState::Available);
        }
        for name in ["climate", "hydrology", "materials", "terrain_generation"] {
            capabilities.insert(name.to_owned(), CapabilityState::NotAvailable);
        }
        Ok(Self {
            schema: SPATIAL_MANIFEST_SCHEMA.to_owned(),
            spatial_release_id: spatial_release_id.into(),
            global_coverage_level: spatial_reference.global_prototype_level,
            spatial_reference,
            identity_format_version: IDENTITY_FORMAT_VERSION.to_owned(),
            face_order: ["+X", "-X", "+Y", "-Y", "+Z", "-Z"].map(str::to_owned),
            boundary_tie_break: "X>Y>Z; positive side owns zero".to_owned(),
            child_order: ["SW", "SE", "NW", "NE"].map(str::to_owned),
            patches,
            capabilities,
            validation_evidence: ValidationEvidence {
                profile: "E1-G01..G14-v1".to_owned(),
                l8_relative_area_tolerance: "1e-12".to_owned(),
                l0_topology: l0_topology_evidence()?,
            },
            direct_dependency_hashes: BTreeMap::new(),
        })
    }
}

fn l0_topology_evidence() -> Result<Vec<TopologyEvidence>, sphere_core::SphereError> {
    let mut evidence = Vec::with_capacity(24);
    for face in sphere_core::CubeFace::ALL {
        let source = CellId::new(face, 0, 0, 0)?;
        for source_edge in CardinalDirection::ALL {
            let relation = source.neighbour_relation(source_edge)?;
            evidence.push(TopologyEvidence {
                source_cell: source,
                source_edge,
                target_cell: relation.target_cell,
                target_edge: relation.target_edge,
                edge_orientation: relation.edge_orientation,
            });
        }
    }
    evidence.sort_by_key(|entry| (entry.source_cell, entry.source_edge));
    Ok(evidence)
}

pub(crate) fn validate_reference(reference: &SpatialReference) -> bool {
    reference.frame_id == SPATIAL_FRAME_ID
        && reference.maximum_identity_level == 30
        && reference.radius_metres.is_finite()
        && reference.radius_metres > 0.0
        && reference.global_prototype_level <= reference.local_prototype_level
        && reference.local_prototype_level <= reference.maximum_identity_level
}
