use crate::SphereError;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;

pub const SPATIAL_FRAME_ID: &str = "cubed-sphere-equiangular-v1";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum CubeFace {
    PosX = 0,
    NegX = 1,
    PosY = 2,
    NegY = 3,
    PosZ = 4,
    NegZ = 5,
}

impl CubeFace {
    pub const ALL: [Self; 6] = [
        Self::PosX,
        Self::NegX,
        Self::PosY,
        Self::NegY,
        Self::PosZ,
        Self::NegZ,
    ];

    pub(crate) fn from_index(index: u8) -> Option<Self> {
        Self::ALL.get(usize::from(index)).copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[repr(u8)]
pub enum CardinalDirection {
    West = 0,
    East = 1,
    South = 2,
    North = 3,
}

impl CardinalDirection {
    pub const ALL: [Self; 4] = [Self::West, Self::East, Self::South, Self::North];
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CellId {
    face: CubeFace,
    level: u8,
    i: u32,
    j: u32,
}

impl CellId {
    /// Creates a cell identity.
    ///
    /// # Errors
    /// Returns [`SphereError::InvalidLevel`] above level 30 or
    /// [`SphereError::InvalidCell`] when either index is outside the level.
    pub fn new(face: CubeFace, level: u8, i: u32, j: u32) -> Result<Self, SphereError> {
        if level > 30 {
            return Err(SphereError::InvalidLevel(level));
        }
        let count = 1_u32 << level;
        if i >= count || j >= count {
            return Err(SphereError::InvalidCell);
        }
        Ok(Self { face, level, i, j })
    }

    #[must_use]
    pub const fn face(self) -> CubeFace {
        self.face
    }
    #[must_use]
    pub const fn level(self) -> u8 {
        self.level
    }
    #[must_use]
    pub const fn i(self) -> u32 {
        self.i
    }
    #[must_use]
    pub const fn j(self) -> u32 {
        self.j
    }

    #[must_use]
    pub fn parent(self) -> Option<Self> {
        (self.level > 0).then(|| Self {
            face: self.face,
            level: self.level - 1,
            i: self.i / 2,
            j: self.j / 2,
        })
    }

    /// Returns four children in the stable order 0, 1, 2, 3.
    ///
    /// # Errors
    /// Returns [`SphereError::InvalidLevel`] when this is an L30 cell.
    pub fn children(self) -> Result<[Self; 4], SphereError> {
        if self.level == 30 {
            return Err(SphereError::InvalidLevel(31));
        }
        let level = self.level + 1;
        let i = self.i * 2;
        let j = self.j * 2;
        Ok([
            Self {
                face: self.face,
                level,
                i,
                j,
            },
            Self {
                face: self.face,
                level,
                i: i + 1,
                j,
            },
            Self {
                face: self.face,
                level,
                i,
                j: j + 1,
            },
            Self {
                face: self.face,
                level,
                i: i + 1,
                j: j + 1,
            },
        ])
    }

    /// Returns the ancestor at `level`.
    ///
    /// # Errors
    /// Returns [`SphereError::InvalidLevel`] when the requested level is deeper.
    pub fn ancestor(self, level: u8) -> Result<Self, SphereError> {
        if level > self.level {
            return Err(SphereError::InvalidLevel(level));
        }
        let shift = self.level - level;
        Self::new(self.face, level, self.i >> shift, self.j >> shift)
    }

    /// Morton path bits only. This is not a globally unique cell identity.
    #[must_use]
    pub fn morton_path(self) -> u64 {
        let mut key = 0_u64;
        for bit in 0..self.level {
            let mask = 1_u32 << bit;
            key |= u64::from((self.i & mask) != 0) << (2 * bit);
            key |= u64::from((self.j & mask) != 0) << (2 * bit + 1);
        }
        key
    }

    #[must_use]
    pub fn path(self) -> String {
        let mut path = String::with_capacity(usize::from(self.level));
        for depth in 0..self.level {
            let bit = self.level - depth - 1;
            let child = ((self.i >> bit) & 1) + 2 * ((self.j >> bit) & 1);
            let digit = match child {
                0 => '0',
                1 => '1',
                2 => '2',
                3 => '3',
                _ => unreachable!("two bits can only encode 0..=3"),
            };
            path.push(digit);
        }
        path
    }

    pub(crate) const fn from_valid_parts(face: CubeFace, level: u8, i: u32, j: u32) -> Self {
        Self { face, level, i, j }
    }
}

impl Display for CellId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "cs1/F{}/L{}/{}",
            self.face as u8,
            self.level,
            self.path()
        )
    }
}

impl FromStr for CellId {
    type Err = SphereError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = value.split('/').collect();
        if parts.len() != 4 || parts[0] != "cs1" {
            return Err(SphereError::InvalidCellIdentity(value.to_owned()));
        }
        let face = parts[1]
            .strip_prefix('F')
            .and_then(|v| v.parse::<u8>().ok())
            .and_then(CubeFace::from_index)
            .ok_or_else(|| SphereError::InvalidCellIdentity(value.to_owned()))?;
        let level = parts[2]
            .strip_prefix('L')
            .and_then(|v| v.parse::<u8>().ok())
            .ok_or_else(|| SphereError::InvalidCellIdentity(value.to_owned()))?;
        if level > 30 || parts[3].len() != usize::from(level) {
            return Err(SphereError::InvalidCellIdentity(value.to_owned()));
        }
        let mut i = 0_u32;
        let mut j = 0_u32;
        for character in parts[3].bytes() {
            if !(b'0'..=b'3').contains(&character) {
                return Err(SphereError::InvalidCellIdentity(value.to_owned()));
            }
            let child = character - b'0';
            i = (i << 1) | u32::from(child & 1);
            j = (j << 1) | u32::from((child >> 1) & 1);
        }
        Self::new(face, level, i, j)
    }
}

impl TryFrom<String> for CellId {
    type Error = SphereError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<CellId> for String {
    fn from(value: CellId) -> Self {
        value.to_string()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct EdgeId {
    first: CellId,
    second: CellId,
}

impl EdgeId {
    pub(crate) fn new(left: CellId, right: CellId) -> Self {
        let (first, second) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        Self { first, second }
    }
    #[must_use]
    pub const fn cells(self) -> [CellId; 2] {
        [self.first, self.second]
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct VertexId {
    incident_cells: Vec<CellId>,
}

impl VertexId {
    pub(crate) fn new(mut incident_cells: Vec<CellId>) -> Self {
        incident_cells.sort_unstable();
        incident_cells.dedup();
        Self { incident_cells }
    }
    #[must_use]
    pub fn incident_cells(&self) -> &[CellId] {
        &self.incident_cells
    }
}
