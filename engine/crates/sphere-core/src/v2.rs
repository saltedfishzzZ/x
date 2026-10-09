//! E2.1对E1空间核心的兼容扩展：可解释查询、边方向和补丁重建。

use crate::{
    CardinalDirection, CellId, CoverageRole, EdgeId, PatchPlan, PatchRequest, SphereError,
    SphereQuery, UnitVec3,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResolutionStatus {
    Exact,
    CoarserThanRequested,
    FinerThanRequested,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellResolution {
    pub requested_level: u8,
    pub actual_level: Option<u8>,
    pub status: ResolutionStatus,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpatialCapability {
    CellIdentity,
    CellGeometry,
    EdgeGeometry,
    NeighbourTopology,
    PatchTwoToOne,
    PatchQueryStatus,
}

impl SpatialCapability {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::CellIdentity => "cell_identity",
            Self::CellGeometry => "cell_geometry",
            Self::EdgeGeometry => "edge_geometry",
            Self::NeighbourTopology => "neighbour_topology",
            Self::PatchTwoToOne => "patch_2_to_1",
            Self::PatchQueryStatus => "patch_query_status",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SpatialQueryStatus {
    Resolved,
    OutsidePublishedCoverage,
    PatchNotLoaded,
    UnsupportedLevel,
    DamagedPatch,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpatialQueryResult {
    pub actual_cell: Option<CellId>,
    pub resolution: CellResolution,
    pub status: SpatialQueryStatus,
    pub coverage_role: Option<CoverageRole>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatchAvailability {
    Absent,
    Loaded(PatchPlan),
    NotLoaded {
        core_parents: BTreeSet<CellId>,
        target_level: u8,
    },
    Damaged {
        core_parents: BTreeSet<CellId>,
        target_level: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeGeometry {
    pub shared_edge_id: EdgeId,
    pub source_cell: CellId,
    pub target_cell: CellId,
    pub midpoint: UnitVec3,
    pub tangent: UnitVec3,
    pub across_normal: UnitVec3,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchValidationFinding {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchValidationReport {
    pub valid: bool,
    pub findings: Vec<PatchValidationFinding>,
}

impl CellId {
    /// Reconstructs shared-edge geometry with the across-edge normal directed
    /// from this cell to its requested neighbour.
    ///
    /// # Errors
    /// Propagates topology or unit-vector construction failures.
    pub fn edge_geometry(self, direction: CardinalDirection) -> Result<EdgeGeometry, SphereError> {
        let relation = self.neighbour_relation(direction)?;
        let [first, second] = relation.shared_edge_id.cells();
        let first_direction = CardinalDirection::ALL
            .into_iter()
            .find(|candidate| first.neighbour(*candidate).is_ok_and(|cell| cell == second))
            .ok_or(SphereError::NotAdjacent)?;
        let endpoints = first.edge_endpoints(first_direction);
        let midpoint = UnitVec3::new(endpoints[0].as_vec3() + endpoints[1].as_vec3())?;
        let tangent = UnitVec3::new(endpoints[1].as_vec3() - endpoints[0].as_vec3())?;
        let delta = second.center() - first.center();
        let canonical_across =
            UnitVec3::new(delta - midpoint.as_vec3() * delta.dot(midpoint.as_vec3()))?;
        let across_normal = if self == first {
            canonical_across
        } else {
            canonical_across.negated()
        };
        Ok(EdgeGeometry {
            shared_edge_id: relation.shared_edge_id,
            source_cell: self,
            target_cell: relation.target_cell,
            midpoint,
            tangent,
            across_normal,
        })
    }
}

impl SphereQuery {
    /// Resolves the actual published leaf and reports why requested detail is
    /// unavailable. The supplied patch state must come from a release validator.
    ///
    /// # Errors
    /// Rejects an invalid position vector.
    pub fn resolve(
        &self,
        vector: crate::Vec3,
        requested_level: u8,
        patch: &PatchAvailability,
    ) -> Result<SpatialQueryResult, SphereError> {
        let reference = self.reference();
        if requested_level > reference.maximum_identity_level {
            return Ok(unavailable(
                requested_level,
                SpatialQueryStatus::UnsupportedLevel,
            ));
        }
        let global = crate::cell_at(vector, reference.global_prototype_level)?;
        if requested_level <= reference.global_prototype_level {
            return Ok(resolved(requested_level, global, CoverageRole::Ambient));
        }
        match patch {
            PatchAvailability::Loaded(plan) => {
                if plan.base_level != reference.global_prototype_level {
                    return Ok(unavailable(
                        requested_level,
                        SpatialQueryStatus::DamagedPatch,
                    ));
                }
                let actual = if plan.core_parents.contains(&global) {
                    Some((
                        crate::cell_at(vector, plan.target_level)?,
                        CoverageRole::Core,
                    ))
                } else {
                    let transition = crate::cell_at(vector, plan.base_level + 1)?;
                    plan.transition_leaves
                        .contains(&transition)
                        .then_some((transition, CoverageRole::Transition))
                };
                Ok(actual.map_or_else(
                    || {
                        unavailable(
                            requested_level,
                            SpatialQueryStatus::OutsidePublishedCoverage,
                        )
                    },
                    |(cell, role)| resolved(requested_level, cell, role),
                ))
            }
            PatchAvailability::NotLoaded {
                core_parents,
                target_level,
            } => Ok(
                if core_parents.contains(&global) && requested_level <= *target_level {
                    unavailable(requested_level, SpatialQueryStatus::PatchNotLoaded)
                } else {
                    unavailable(
                        requested_level,
                        SpatialQueryStatus::OutsidePublishedCoverage,
                    )
                },
            ),
            PatchAvailability::Damaged {
                core_parents,
                target_level,
            } => Ok(
                if core_parents.contains(&global) && requested_level <= *target_level {
                    unavailable(requested_level, SpatialQueryStatus::DamagedPatch)
                } else {
                    unavailable(
                        requested_level,
                        SpatialQueryStatus::OutsidePublishedCoverage,
                    )
                },
            ),
            PatchAvailability::Absent => Ok(unavailable(
                requested_level,
                SpatialQueryStatus::OutsidePublishedCoverage,
            )),
        }
    }
}

impl PatchPlan {
    #[must_use]
    pub fn active_leaves(&self) -> BTreeSet<CellId> {
        self.core.union(&self.transition_leaves).copied().collect()
    }

    /// Rebuilds every derived patch set from its authoritative request.
    #[must_use]
    pub fn validate_against_request(&self, request: PatchRequest) -> PatchValidationReport {
        let mut findings = Vec::new();
        match Self::build(request) {
            Ok(expected) => {
                compare_set(
                    "PATCH_CORE_MISMATCH",
                    &self.core,
                    &expected.core,
                    &mut findings,
                );
                compare_set(
                    "PATCH_TRANSITION_MISMATCH",
                    &self.transition_leaves,
                    &expected.transition_leaves,
                    &mut findings,
                );
                compare_set(
                    "PATCH_HALO_MISMATCH",
                    &self.halo,
                    &expected.halo,
                    &mut findings,
                );
                if self.patch_id != expected.patch_id
                    || self.base_level != expected.base_level
                    || self.target_level != expected.target_level
                    || self.core_parents != expected.core_parents
                {
                    findings.push(PatchValidationFinding {
                        code: "PATCH_METADATA_MISMATCH".to_owned(),
                        detail: "stored patch metadata differs from the reconstructed plan"
                            .to_owned(),
                    });
                }
                validate_leaf_ancestry(&self.active_leaves(), &mut findings);
            }
            Err(error) => findings.push(PatchValidationFinding {
                code: "PATCH_REQUEST_INVALID".to_owned(),
                detail: error.to_string(),
            }),
        }
        PatchValidationReport {
            valid: findings.is_empty(),
            findings,
        }
    }
}

fn resolved(requested_level: u8, cell: CellId, role: CoverageRole) -> SpatialQueryResult {
    let status = match cell.level().cmp(&requested_level) {
        std::cmp::Ordering::Less => ResolutionStatus::CoarserThanRequested,
        std::cmp::Ordering::Equal => ResolutionStatus::Exact,
        std::cmp::Ordering::Greater => ResolutionStatus::FinerThanRequested,
    };
    SpatialQueryResult {
        actual_cell: Some(cell),
        resolution: CellResolution {
            requested_level,
            actual_level: Some(cell.level()),
            status,
        },
        status: SpatialQueryStatus::Resolved,
        coverage_role: Some(role),
    }
}

const fn unavailable(requested_level: u8, status: SpatialQueryStatus) -> SpatialQueryResult {
    SpatialQueryResult {
        actual_cell: None,
        resolution: CellResolution {
            requested_level,
            actual_level: None,
            status: ResolutionStatus::Unavailable,
        },
        status,
        coverage_role: None,
    }
}

fn compare_set(
    code: &str,
    actual: &BTreeSet<CellId>,
    expected: &BTreeSet<CellId>,
    findings: &mut Vec<PatchValidationFinding>,
) {
    if actual != expected {
        findings.push(PatchValidationFinding {
            code: code.to_owned(),
            detail: "stored cells differ from the authoritative request reconstruction".to_owned(),
        });
    }
}

fn validate_leaf_ancestry(leaves: &BTreeSet<CellId>, findings: &mut Vec<PatchValidationFinding>) {
    for leaf in leaves {
        let mut ancestor = leaf.parent();
        while let Some(parent) = ancestor {
            if leaves.contains(&parent) {
                findings.push(PatchValidationFinding {
                    code: "PATCH_LEAF_ANCESTRY_OVERLAP".to_owned(),
                    detail: format!("{parent} is both a leaf and an ancestor of {leaf}"),
                });
                return;
            }
            ancestor = parent.parent();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CubeFace, SpatialReference, Vec3};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct CrossFacePatchFixture {
        fixture_id: String,
        schema: String,
        request: PatchRequest,
        expect_multiple_faces: bool,
    }

    fn fixture() -> (PatchRequest, PatchPlan) {
        let parent = CellId::new(CubeFace::PosX, 8, 255, 128).unwrap();
        let request = PatchRequest {
            patch_id: "edge-patch".to_owned(),
            core_parents: vec![parent],
            target_level: 10,
            halo_parent_rings: 1,
            cell_budget: 128,
        };
        let plan = PatchPlan::build(request.clone()).unwrap();
        (request, plan)
    }

    #[test]
    fn reverse_edge_geometry_has_stable_identity_and_opposite_normal() {
        let source = CellId::new(CubeFace::PosX, 6, 63, 63).unwrap();
        for direction in CardinalDirection::ALL {
            let forward = source.edge_geometry(direction).unwrap();
            let reverse_direction = CardinalDirection::ALL
                .into_iter()
                .find(|candidate| {
                    forward
                        .target_cell
                        .neighbour(*candidate)
                        .is_ok_and(|cell| cell == source)
                })
                .unwrap();
            let reverse = forward
                .target_cell
                .edge_geometry(reverse_direction)
                .unwrap();
            assert_eq!(forward.shared_edge_id, reverse.shared_edge_id);
            assert_eq!(forward.midpoint, reverse.midpoint);
            assert_eq!(
                forward.across_normal.as_vec3(),
                -reverse.across_normal.as_vec3()
            );
        }
    }

    #[test]
    fn request_reconstruction_rejects_deleted_and_forged_regions() {
        let (request, plan) = fixture();
        assert!(plan.validate_against_request(request.clone()).valid);

        let mut deleted = plan.clone();
        deleted.transition_leaves.pop_first();
        assert_eq!(
            deleted.validate_against_request(request.clone()).findings[0].code,
            "PATCH_TRANSITION_MISMATCH"
        );

        let mut forged = plan;
        forged.halo.clear();
        assert_eq!(
            forged.validate_against_request(request).findings[0].code,
            "PATCH_HALO_MISMATCH"
        );
    }

    #[test]
    fn cross_face_patch_fixture_is_executable() {
        let fixture: CrossFacePatchFixture = serde_json::from_slice(include_bytes!(
            "../../../fixtures/e2.1/spatial/E21S01-cross-face-patch.json"
        ))
        .unwrap();
        assert_eq!(fixture.fixture_id, "E21-S01");
        assert_eq!(fixture.schema, "e2.1-cross-face-patch-fixture-v1");
        let request = fixture.request.clone();
        let plan = PatchPlan::build(fixture.request).unwrap();
        assert!(plan.validate_against_request(request).valid);
        let faces = plan
            .core
            .iter()
            .chain(&plan.transition_leaves)
            .chain(&plan.halo)
            .map(|cell| cell.face())
            .collect::<BTreeSet<_>>();
        assert_eq!(faces.len() > 1, fixture.expect_multiple_faces);
    }

    #[test]
    fn query_states_are_explicit() {
        let (_, plan) = fixture();
        let reference = SpatialReference::prototype().unwrap();
        let query = SphereQuery::new(reference, None);
        let point = plan.core_parents.first().unwrap().center();
        let loaded = query
            .resolve(point, 10, &PatchAvailability::Loaded(plan.clone()))
            .unwrap();
        assert_eq!(loaded.status, SpatialQueryStatus::Resolved);
        assert_eq!(loaded.resolution.status, ResolutionStatus::Exact);

        let transition_point = plan.transition_leaves.first().unwrap().center();
        let transition = query
            .resolve(
                transition_point,
                10,
                &PatchAvailability::Loaded(plan.clone()),
            )
            .unwrap();
        assert_eq!(transition.status, SpatialQueryStatus::Resolved);
        assert_eq!(
            transition.resolution.status,
            ResolutionStatus::CoarserThanRequested
        );
        assert_eq!(transition.actual_cell.unwrap().level(), 9);

        let parents = plan.core_parents.clone();
        assert_eq!(
            query
                .resolve(
                    point,
                    10,
                    &PatchAvailability::NotLoaded {
                        core_parents: parents.clone(),
                        target_level: 10,
                    },
                )
                .unwrap()
                .status,
            SpatialQueryStatus::PatchNotLoaded
        );
        assert_eq!(
            query
                .resolve(
                    point,
                    10,
                    &PatchAvailability::Damaged {
                        core_parents: parents,
                        target_level: 10,
                    },
                )
                .unwrap()
                .status,
            SpatialQueryStatus::DamagedPatch
        );
        assert_eq!(
            query
                .resolve(Vec3::new(-1.0, 0.0, 0.0), 10, &PatchAvailability::Absent)
                .unwrap()
                .status,
            SpatialQueryStatus::OutsidePublishedCoverage
        );
        assert_eq!(
            query
                .resolve(point, 31, &PatchAvailability::Loaded(plan))
                .unwrap()
                .status,
            SpatialQueryStatus::UnsupportedLevel
        );
    }

    #[test]
    fn face_seams_and_cube_corners_resolve_once_by_the_cs1_tie_rule() {
        let query = SphereQuery::new(SpatialReference::prototype().unwrap(), None);
        for point in [
            Vec3::new(1.0, 1.0, 0.25),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-1.0, -1.0, -1.0),
        ] {
            let first = query.resolve(point, 8, &PatchAvailability::Absent).unwrap();
            let second = query.resolve(point, 8, &PatchAvailability::Absent).unwrap();
            assert_eq!(first, second);
            assert_eq!(first.status, SpatialQueryStatus::Resolved);
            assert!(first.actual_cell.is_some());
        }
    }
}
