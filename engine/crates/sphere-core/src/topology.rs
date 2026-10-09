use crate::mapping::vector_on_face;
use crate::{
    CardinalDirection, CellId, CubeFace, EdgeId, FaceCoord, SphereError, UnitVec3, VertexId,
    cell_at, face_to_vector,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EdgeOrientation {
    Same,
    Reversed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NeighbourRelation {
    pub source_cell: CellId,
    pub source_edge: CardinalDirection,
    pub target_cell: CellId,
    pub target_edge: CardinalDirection,
    pub edge_orientation: EdgeOrientation,
    pub shared_edge_id: EdgeId,
}

impl CellId {
    /// Returns the geometrically adjacent cell, including across cube faces.
    ///
    /// # Errors
    /// Propagates invalid-level or mapping errors.
    pub fn neighbour(self, direction: CardinalDirection) -> Result<Self, SphereError> {
        let count_u32 = 1_u32 << self.level();
        let count = f64::from(count_u32);
        match direction {
            CardinalDirection::West if self.i() > 0 => {
                return Self::new(self.face(), self.level(), self.i() - 1, self.j());
            }
            CardinalDirection::East if self.i() + 1 < count_u32 => {
                return Self::new(self.face(), self.level(), self.i() + 1, self.j());
            }
            CardinalDirection::South if self.j() > 0 => {
                return Self::new(self.face(), self.level(), self.i(), self.j() - 1);
            }
            CardinalDirection::North if self.j() + 1 < count_u32 => {
                return Self::new(self.face(), self.level(), self.i(), self.j() + 1);
            }
            _ => {}
        }
        let step = 2.0 / count;
        let mut s = -1.0 + 2.0 * (f64::from(self.i()) + 0.5) / count;
        let mut t = -1.0 + 2.0 * (f64::from(self.j()) + 0.5) / count;
        match direction {
            CardinalDirection::West => s -= step,
            CardinalDirection::East => s += step,
            CardinalDirection::South => t -= step,
            CardinalDirection::North => t += step,
        }
        cell_at(
            face_to_vector(FaceCoord {
                face: self.face(),
                s,
                t,
            }),
            self.level(),
        )
    }

    /// Returns all four neighbours in stable edge order.
    ///
    /// # Errors
    /// Propagates an unexpected mapping error.
    pub fn neighbours(self) -> Result<[Self; 4], SphereError> {
        Ok([
            self.neighbour(CardinalDirection::West)?,
            self.neighbour(CardinalDirection::East)?,
            self.neighbour(CardinalDirection::South)?,
            self.neighbour(CardinalDirection::North)?,
        ])
    }

    /// Returns the full reciprocal edge relation.
    ///
    /// # Errors
    /// Returns [`SphereError::NotAdjacent`] if reciprocity cannot be found.
    pub fn neighbour_relation(
        self,
        direction: CardinalDirection,
    ) -> Result<NeighbourRelation, SphereError> {
        let target = self.neighbour(direction)?;
        let target_edge = CardinalDirection::ALL
            .into_iter()
            .find(|candidate| target.neighbour(*candidate).is_ok_and(|cell| cell == self))
            .ok_or(SphereError::NotAdjacent)?;
        let source_points = self.edge_endpoints(direction);
        let target_points = target.edge_endpoints(target_edge);
        let same = nearly_same(source_points[0], target_points[0])
            && nearly_same(source_points[1], target_points[1]);
        let reversed = nearly_same(source_points[0], target_points[1])
            && nearly_same(source_points[1], target_points[0]);
        let edge_orientation = if same {
            EdgeOrientation::Same
        } else if reversed {
            EdgeOrientation::Reversed
        } else {
            return Err(SphereError::NotAdjacent);
        };
        Ok(NeighbourRelation {
            source_cell: self,
            source_edge: direction,
            target_cell: target,
            target_edge,
            edge_orientation,
            shared_edge_id: EdgeId::new(self, target),
        })
    }

    /// Returns the canonical identity for one of the four corners.
    ///
    /// Corner order is southwest, southeast, northeast, northwest.
    ///
    /// # Errors
    /// Returns [`SphereError::InvalidCell`] for a corner index above three.
    pub fn vertex_id(self, corner: u8) -> Result<VertexId, SphereError> {
        let vertex = *self
            .corner_units()
            .get(usize::from(corner))
            .ok_or(SphereError::InvalidCell)?;
        Ok(VertexId::new(incident_cells(vertex, self.level())))
    }
}

fn nearly_same(left: UnitVec3, right: UnitVec3) -> bool {
    left.cross(right).norm() <= 2.0e-14 && left.dot(right) > 0.0
}

fn incident_cells(vertex: UnitVec3, level: u8) -> Vec<CellId> {
    let count = 1_u32 << level;
    let count_f64 = f64::from(count);
    let mut cells = Vec::new();
    for face in CubeFace::ALL {
        let Some((s, t)) = vector_on_face(face, vertex) else {
            continue;
        };
        let x = ((s + 1.0) * 0.5 * count_f64).clamp(0.0, count_f64);
        let y = ((t + 1.0) * 0.5 * count_f64).clamp(0.0, count_f64);
        let ix = x.round();
        let iy = y.round();
        if (x - ix).abs() > 1.0e-8 || (y - iy).abs() > 1.0e-8 {
            continue;
        }
        #[allow(clippy::cast_possible_truncation)]
        let ix = ix as i64;
        #[allow(clippy::cast_possible_truncation)]
        let iy = iy as i64;
        for i in [ix - 1, ix] {
            for j in [iy - 1, iy] {
                if i >= 0 && j >= 0 && i < i64::from(count) && j < i64::from(count) {
                    let (Ok(cell_i), Ok(cell_j)) = (u32::try_from(i), u32::try_from(j)) else {
                        continue;
                    };
                    cells.push(CellId::from_valid_parts(face, level, cell_i, cell_j));
                }
            }
        }
    }
    cells
}
