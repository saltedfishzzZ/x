use crate::manifest::{IDENTITY_FORMAT_VERSION, SPATIAL_MANIFEST_SCHEMA, SpatialReleaseManifest};
use crate::{ReleaseError, validate_release_id};
use sphere_core::CardinalDirection;
use std::collections::BTreeSet;
use world_contracts::CapabilityState;

/// Validates all reconstructable and discrete facts in a spatial manifest.
///
/// # Errors
/// Returns a precise release error when the schema, topology, capabilities,
/// patch lineage, or area evidence is inconsistent.
pub fn validate_manifest(manifest: &SpatialReleaseManifest) -> Result<(), ReleaseError> {
    validate_release_id(&manifest.spatial_release_id)?;
    if manifest.schema != SPATIAL_MANIFEST_SCHEMA {
        return Err(ReleaseError::InvalidManifest("unsupported schema".into()));
    }
    if manifest.identity_format_version != IDENTITY_FORMAT_VERSION {
        return Err(ReleaseError::InvalidManifest(
            "unsupported identity format".into(),
        ));
    }
    if !crate::manifest::validate_reference(&manifest.spatial_reference)
        || manifest.global_coverage_level != manifest.spatial_reference.global_prototype_level
    {
        return Err(ReleaseError::InvalidManifest(
            "invalid spatial reference".into(),
        ));
    }
    if manifest.face_order != ["+X", "-X", "+Y", "-Y", "+Z", "-Z"]
        || manifest.child_order != ["SW", "SE", "NW", "NE"]
        || manifest.boundary_tie_break != "X>Y>Z; positive side owns zero"
    {
        return Err(ReleaseError::InvalidManifest(
            "identity rules do not match cs1".into(),
        ));
    }
    validate_capabilities(manifest)?;
    validate_patches(manifest)?;
    validate_topology(manifest)?;
    let recomputed = sphere_core::unit_sphere_area_error(8)?;
    if manifest.validation_evidence.l8_relative_area_tolerance != "1e-12" {
        return Err(ReleaseError::InvalidManifest(
            "area tolerance does not match the E1 profile".into(),
        ));
    }
    let tolerance = manifest
        .validation_evidence
        .l8_relative_area_tolerance
        .parse::<f64>()
        .map_err(|_| ReleaseError::InvalidManifest("area tolerance is not numeric".into()))?;
    if manifest.validation_evidence.profile != "E1-G01..G14-v1"
        || !tolerance.is_finite()
        || recomputed > tolerance
    {
        return Err(ReleaseError::InvalidManifest(
            "area validation evidence is inconsistent".into(),
        ));
    }
    Ok(())
}

fn validate_capabilities(manifest: &SpatialReleaseManifest) -> Result<(), ReleaseError> {
    for capability in [
        "cell_identity",
        "cell_geometry",
        "fixed_level_cap_cover",
        "neighbour_topology",
        "patch_2_to_1",
    ] {
        if manifest.capabilities.get(capability) != Some(&CapabilityState::Available) {
            return Err(ReleaseError::InvalidManifest(format!(
                "required capability is unavailable: {capability}"
            )));
        }
    }
    Ok(())
}

fn validate_patches(manifest: &SpatialReleaseManifest) -> Result<(), ReleaseError> {
    if manifest
        .patches
        .windows(2)
        .any(|pair| pair[0].patch_id >= pair[1].patch_id)
    {
        return Err(ReleaseError::InvalidManifest(
            "patch identities are duplicated or unsorted".into(),
        ));
    }
    for patch in &manifest.patches {
        if patch.target_level != patch.base_level + 2
            || patch
                .core
                .iter()
                .any(|cell| cell.level() != patch.target_level)
            || patch
                .transition_leaves
                .iter()
                .any(|cell| cell.level() != patch.base_level + 1)
            || patch
                .halo
                .iter()
                .any(|cell| cell.level() != patch.target_level)
            || patch.core.iter().any(|cell| patch.halo.contains(cell))
        {
            return Err(ReleaseError::InvalidManifest(format!(
                "invalid patch hierarchy: {}",
                patch.patch_id
            )));
        }
    }
    Ok(())
}

fn validate_topology(manifest: &SpatialReleaseManifest) -> Result<(), ReleaseError> {
    if manifest.validation_evidence.l0_topology.len() != 24 {
        return Err(ReleaseError::InvalidManifest(
            "L0 topology must contain 24 directed edges".into(),
        ));
    }
    let mut identities = BTreeSet::new();
    for evidence in &manifest.validation_evidence.l0_topology {
        if evidence.source_cell.level() != 0
            || !identities.insert((evidence.source_cell, evidence.source_edge))
        {
            return Err(ReleaseError::InvalidManifest(
                "L0 topology contains duplicate or non-L0 source".into(),
            ));
        }
        let expected = evidence
            .source_cell
            .neighbour_relation(evidence.source_edge)?;
        if expected.target_cell != evidence.target_cell
            || expected.target_edge != evidence.target_edge
            || expected.edge_orientation != evidence.edge_orientation
        {
            return Err(ReleaseError::InvalidManifest(
                "L0 topology evidence does not match geometry".into(),
            ));
        }
    }
    for face in sphere_core::CubeFace::ALL {
        let source = sphere_core::CellId::new(face, 0, 0, 0)?;
        for edge in CardinalDirection::ALL {
            if !identities.contains(&(source, edge)) {
                return Err(ReleaseError::InvalidManifest(
                    "L0 topology is incomplete".into(),
                ));
            }
        }
    }
    Ok(())
}
