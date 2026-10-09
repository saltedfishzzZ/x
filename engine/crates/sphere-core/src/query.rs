use crate::{
    CellGeometry, CellId, CoverageRole, PatchPlan, SpatialReference, SphereError, UnitVec3, Vec3,
    cell_at,
};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphericalCap {
    pub center: UnitVec3,
    pub angular_radius: f64,
}

impl SphericalCap {
    /// Creates a cap with radius in radians.
    ///
    /// # Errors
    /// Rejects non-finite radii and values outside [0, pi].
    pub fn new(center: UnitVec3, angular_radius: f64) -> Result<Self, SphereError> {
        if !angular_radius.is_finite() || !(0.0..=PI).contains(&angular_radius) {
            return Err(SphereError::InvalidAngularRadius);
        }
        Ok(Self {
            center,
            angular_radius,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SphereQuery {
    reference: SpatialReference,
    patch: Option<PatchPlan>,
}

impl SphereQuery {
    #[must_use]
    pub fn new(reference: SpatialReference, patch: Option<PatchPlan>) -> Self {
        Self { reference, patch }
    }

    #[must_use]
    pub const fn reference(&self) -> &SpatialReference {
        &self.reference
    }

    /// Locates a point at an explicit level.
    ///
    /// # Errors
    /// Propagates invalid vectors or levels.
    pub fn locate(&self, vector: Vec3, level: u8) -> Result<CellId, SphereError> {
        cell_at(vector, level)
    }

    /// Returns geometry under the query's spatial reference.
    ///
    /// # Errors
    /// Propagates geometry errors.
    pub fn geometry(&self, cell: CellId) -> Result<CellGeometry, SphereError> {
        cell.geometry(&self.reference)
    }

    #[must_use]
    pub fn patch_role(&self, cell: CellId) -> CoverageRole {
        self.patch
            .as_ref()
            .map_or(CoverageRole::Ambient, |patch| patch.coverage_role(cell))
    }

    /// Resolves the active leaf for a point.
    ///
    /// # Errors
    /// Propagates point-location errors.
    pub fn active_cell(&self, vector: Vec3) -> Result<CellId, SphereError> {
        let base = cell_at(vector, self.reference.global_prototype_level)?;
        let Some(patch) = &self.patch else {
            return Ok(base);
        };
        if patch.core_parents.contains(&base) {
            return cell_at(vector, patch.target_level);
        }
        let transition = cell_at(vector, patch.base_level + 1)?;
        if patch.transition_leaves.contains(&transition) {
            return Ok(transition);
        }
        Ok(base)
    }

    /// Conservatively covers a spherical cap at a fixed level.
    ///
    /// # Errors
    /// Rejects levels above 30.
    pub fn cover_cap(&self, cap: SphericalCap, level: u8) -> Result<Vec<CellId>, SphereError> {
        if level > 30 {
            return Err(SphereError::InvalidLevel(level));
        }
        let count = 1_u32 << level;
        let mut result = Vec::new();
        for face in crate::CubeFace::ALL {
            for j in 0..count {
                for i in 0..count {
                    let cell = CellId::from_valid_parts(face, level, i, j);
                    let center = cell.center_unit();
                    let bound = cell
                        .corner_units()
                        .into_iter()
                        .map(|corner| angular_distance(center, corner))
                        .fold(0.0, f64::max);
                    if angular_distance(cap.center, center) <= cap.angular_radius + bound {
                        result.push(cell);
                    }
                }
            }
        }
        result.sort_unstable();
        Ok(result)
    }
}

fn angular_distance(left: UnitVec3, right: UnitVec3) -> f64 {
    left.cross(right).norm().atan2(left.dot(right))
}
