//! E1 球面空间核心：稳定身份、等角立方球面、拓扑、补丁与查询。

mod geometry;
mod identity;
mod mapping;
mod patch;
mod query;
mod rotation;
mod topology;
mod v2;
mod vector;

pub use geometry::{
    CellGeometry, SpatialReference, TangentFrame, global_unit_area, great_circle_distance,
    spherical_triangle_area, unit_sphere_area_error,
};
pub use identity::{CardinalDirection, CellId, CubeFace, EdgeId, SPATIAL_FRAME_ID, VertexId};
pub use mapping::{FaceCoord, cell_at, face_to_unit_vector, face_to_vector, vector_to_face};
pub use patch::{AggregationWeight, CoverageRole, PatchPlan, PatchRequest};
pub use query::{SphereQuery, SphericalCap};
pub use rotation::{CubeRotation, cube_rotations};
pub use topology::{EdgeOrientation, NeighbourRelation};
pub use v2::{
    CellResolution, EdgeGeometry, PatchAvailability, PatchValidationFinding, PatchValidationReport,
    ResolutionStatus, SpatialCapability, SpatialQueryResult, SpatialQueryStatus,
};
pub use vector::{UnitVec3, Vec3};

use thiserror::Error;

#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum SphereError {
    #[error("zero or non-finite vector cannot identify a sphere position")]
    InvalidVector,
    #[error("quadtree level {0} exceeds the supported limit")]
    InvalidLevel(u8),
    #[error("cell index is outside its level")]
    InvalidCell,
    #[error("invalid cell identity: {0}")]
    InvalidCellIdentity(String),
    #[error("sphere radius must be finite and positive")]
    InvalidRadius,
    #[error("angular radius must be finite and within [0, pi]")]
    InvalidAngularRadius,
    #[error("cells do not share an edge")]
    NotAdjacent,
    #[error("patch request is invalid: {0}")]
    InvalidPatch(String),
    #[error("patch cell budget exceeded: required {required}, budget {budget}")]
    PatchBudgetExceeded { required: usize, budget: usize },
}
