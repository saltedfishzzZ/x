use crate::{
    CardinalDirection, CellId, CubeFace, FaceCoord, SPATIAL_FRAME_ID, SphereError, UnitVec3, Vec3,
    face_to_unit_vector,
};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpatialReference {
    pub frame_id: String,
    pub radius_metres: f64,
    pub maximum_identity_level: u8,
    pub global_prototype_level: u8,
    pub local_prototype_level: u8,
}

impl SpatialReference {
    /// Creates the frozen `prototype-grid-v1` spatial reference.
    ///
    /// # Errors
    /// Returns [`SphereError::InvalidRadius`] for a non-positive radius.
    pub fn prototype() -> Result<Self, SphereError> {
        Self::new(6_371_000.0, 8, 10)
    }

    /// Creates a cs1 spatial reference.
    ///
    /// # Errors
    /// Returns a radius or level error for invalid parameters.
    pub fn new(radius_metres: f64, global_level: u8, local_level: u8) -> Result<Self, SphereError> {
        if !radius_metres.is_finite() || radius_metres <= 0.0 {
            return Err(SphereError::InvalidRadius);
        }
        if global_level > 30 {
            return Err(SphereError::InvalidLevel(global_level));
        }
        if local_level > 30 || local_level < global_level {
            return Err(SphereError::InvalidLevel(local_level));
        }
        Ok(Self {
            frame_id: SPATIAL_FRAME_ID.to_owned(),
            radius_metres,
            maximum_identity_level: 30,
            global_prototype_level: global_level,
            local_prototype_level: local_level,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TangentFrame {
    pub normal: UnitVec3,
    pub u_axis: UnitVec3,
    pub v_axis: UnitVec3,
}

impl TangentFrame {
    /// Builds a deterministic right-handed tangent frame.
    ///
    /// # Errors
    /// Propagates an unexpected unit-vector construction error.
    pub fn at(normal: UnitVec3) -> Result<Self, SphereError> {
        let n = normal.as_vec3();
        let reference = if n.z.abs() < 0.9 {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        let u_axis = UnitVec3::new(reference.cross(n))?;
        let v_axis = UnitVec3::new(n.cross(u_axis.as_vec3()))?;
        Ok(Self {
            normal,
            u_axis,
            v_axis,
        })
    }

    #[must_use]
    pub fn project(self, vector: Vec3) -> [f64; 2] {
        [
            vector.dot(self.u_axis.as_vec3()),
            vector.dot(self.v_axis.as_vec3()),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellGeometry {
    pub cell_id: CellId,
    pub center: UnitVec3,
    pub corners: [UnitVec3; 4],
    pub unit_sphere_area: f64,
    pub area_square_metres: f64,
    pub edge_lengths_metres: [f64; 4],
    pub neighbour_distances_metres: [f64; 4],
    pub tangent_frame: TangentFrame,
}

impl CellId {
    #[must_use]
    pub fn center(self) -> Vec3 {
        self.center_unit().as_vec3()
    }

    #[must_use]
    pub fn center_unit(self) -> UnitVec3 {
        let count = f64::from(1_u32 << self.level());
        let s = -1.0 + 2.0 * (f64::from(self.i()) + 0.5) / count;
        let t = -1.0 + 2.0 * (f64::from(self.j()) + 0.5) / count;
        unit_from_cell_coord(FaceCoord {
            face: self.face(),
            s,
            t,
        })
    }

    #[must_use]
    pub fn corners(self) -> [Vec3; 4] {
        self.corner_units().map(UnitVec3::as_vec3)
    }

    #[must_use]
    pub fn corner_units(self) -> [UnitVec3; 4] {
        let count = f64::from(1_u32 << self.level());
        let s0 = -1.0 + 2.0 * f64::from(self.i()) / count;
        let s1 = -1.0 + 2.0 * f64::from(self.i() + 1) / count;
        let t0 = -1.0 + 2.0 * f64::from(self.j()) / count;
        let t1 = -1.0 + 2.0 * f64::from(self.j() + 1) / count;
        [
            unit_from_cell_coord(FaceCoord {
                face: self.face(),
                s: s0,
                t: t0,
            }),
            unit_from_cell_coord(FaceCoord {
                face: self.face(),
                s: s1,
                t: t0,
            }),
            unit_from_cell_coord(FaceCoord {
                face: self.face(),
                s: s1,
                t: t1,
            }),
            unit_from_cell_coord(FaceCoord {
                face: self.face(),
                s: s0,
                t: t1,
            }),
        ]
    }

    #[must_use]
    pub fn edge_endpoints(self, direction: CardinalDirection) -> [UnitVec3; 2] {
        let corners = self.corner_units();
        match direction {
            CardinalDirection::West => [corners[0], corners[3]],
            CardinalDirection::East => [corners[1], corners[2]],
            CardinalDirection::South => [corners[0], corners[1]],
            CardinalDirection::North => [corners[3], corners[2]],
        }
    }

    #[must_use]
    pub fn unit_sphere_area(self) -> f64 {
        let [a, b, c, d] = self.corner_units();
        spherical_triangle_area(a.as_vec3(), b.as_vec3(), c.as_vec3())
            + spherical_triangle_area(a.as_vec3(), c.as_vec3(), d.as_vec3())
    }

    /// Computes complete geometry for a spatial reference.
    ///
    /// # Errors
    /// Propagates topology or tangent-frame errors.
    pub fn geometry(self, reference: &SpatialReference) -> Result<CellGeometry, SphereError> {
        let center = self.center_unit();
        let corners = self.corner_units();
        let unit_area = self.unit_sphere_area();
        let mut edge_lengths = [0.0; 4];
        let mut neighbour_distances = [0.0; 4];
        for (index, direction) in CardinalDirection::ALL.into_iter().enumerate() {
            let endpoints = self.edge_endpoints(direction);
            edge_lengths[index] = great_circle_distance(
                endpoints[0].as_vec3(),
                endpoints[1].as_vec3(),
                reference.radius_metres,
            );
            let neighbour = self.neighbour(direction)?;
            neighbour_distances[index] = great_circle_distance(
                center.as_vec3(),
                neighbour.center(),
                reference.radius_metres,
            );
        }
        Ok(CellGeometry {
            cell_id: self,
            center,
            corners,
            unit_sphere_area: unit_area,
            area_square_metres: unit_area * reference.radius_metres * reference.radius_metres,
            edge_lengths_metres: edge_lengths,
            neighbour_distances_metres: neighbour_distances,
            tangent_frame: TangentFrame::at(center)?,
        })
    }
}

fn unit_from_cell_coord(coord: FaceCoord) -> UnitVec3 {
    match face_to_unit_vector(coord) {
        Ok(vector) => vector,
        Err(_) => unreachable!("validated cell coordinates are finite"),
    }
}

#[must_use]
pub fn great_circle_distance(a: Vec3, b: Vec3, radius: f64) -> f64 {
    radius * a.cross(b).norm().atan2(a.dot(b))
}

#[must_use]
pub fn spherical_triangle_area(a: Vec3, b: Vec3, c: Vec3) -> f64 {
    let numerator = a.dot(b.cross(c)).abs();
    let denominator = 1.0 + a.dot(b) + b.dot(c) + c.dot(a);
    2.0 * numerator.atan2(denominator)
}

/// Sums every cell area at a level on the unit sphere.
///
/// # Errors
/// Returns [`SphereError::InvalidLevel`] above level 30.
pub fn global_unit_area(level: u8) -> Result<f64, SphereError> {
    if level > 30 {
        return Err(SphereError::InvalidLevel(level));
    }
    let count = 1_u32 << level;
    let mut sum = 0.0;
    let mut correction = 0.0;
    for face in CubeFace::ALL {
        for j in 0..count {
            for i in 0..count {
                let area = CellId::from_valid_parts(face, level, i, j).unit_sphere_area();
                let adjusted = area - correction;
                let next = sum + adjusted;
                correction = (next - sum) - adjusted;
                sum = next;
            }
        }
    }
    Ok(sum)
}

/// Computes relative global area closure error.
///
/// # Errors
/// Returns [`SphereError::InvalidLevel`] above level 30.
pub fn unit_sphere_area_error(level: u8) -> Result<f64, SphereError> {
    Ok((global_unit_area(level)? - 4.0 * PI).abs() / (4.0 * PI))
}
