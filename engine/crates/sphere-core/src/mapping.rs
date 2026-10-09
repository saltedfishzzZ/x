use crate::{CellId, CubeFace, SphereError, UnitVec3, Vec3};
use std::f64::consts::FRAC_PI_4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceCoord {
    pub face: CubeFace,
    /// Equiangular coordinate in [-1, 1].
    pub s: f64,
    /// Equiangular coordinate in [-1, 1].
    pub t: f64,
}

/// Maps a face coordinate to an authoritative unit vector.
///
/// # Errors
/// Returns [`SphereError::InvalidVector`] for non-finite coordinates.
pub fn face_to_unit_vector(coord: FaceCoord) -> Result<UnitVec3, SphereError> {
    if !coord.s.is_finite() || !coord.t.is_finite() {
        return Err(SphereError::InvalidVector);
    }
    UnitVec3::new(raw_face_vector(coord))
}

fn raw_face_vector(coord: FaceCoord) -> Vec3 {
    let u = (FRAC_PI_4 * coord.s).tan();
    let v = (FRAC_PI_4 * coord.t).tan();
    match coord.face {
        CubeFace::PosX => Vec3::new(1.0, u, v),
        CubeFace::NegX => Vec3::new(-1.0, -u, v),
        CubeFace::PosY => Vec3::new(-u, 1.0, v),
        CubeFace::NegY => Vec3::new(u, -1.0, v),
        CubeFace::PosZ => Vec3::new(u, v, 1.0),
        CubeFace::NegZ => Vec3::new(u, -v, -1.0),
    }
}

/// Compatibility helper returning the underlying vector.
#[must_use]
pub fn face_to_vector(coord: FaceCoord) -> Vec3 {
    raw_face_vector(coord).normalized()
}

/// Maps a non-zero finite vector to its canonical cube face.
///
/// Ties use the frozen priority X, then Y, then Z.
///
/// # Errors
/// Returns [`SphereError::InvalidVector`] for zero or non-finite input.
pub fn vector_to_face(vector: Vec3) -> Result<FaceCoord, SphereError> {
    let p = UnitVec3::new(vector)?.as_vec3();
    let ax = p.x.abs();
    let ay = p.y.abs();
    let az = p.z.abs();
    let (face, u, v) = if ax >= ay && ax >= az {
        if p.x >= 0.0 {
            (CubeFace::PosX, p.y / ax, p.z / ax)
        } else {
            (CubeFace::NegX, -p.y / ax, p.z / ax)
        }
    } else if ay >= az {
        if p.y >= 0.0 {
            (CubeFace::PosY, -p.x / ay, p.z / ay)
        } else {
            (CubeFace::NegY, p.x / ay, p.z / ay)
        }
    } else if p.z >= 0.0 {
        (CubeFace::PosZ, p.x / az, p.y / az)
    } else {
        (CubeFace::NegZ, p.x / az, -p.y / az)
    };
    Ok(FaceCoord {
        face,
        s: u.atan() / FRAC_PI_4,
        t: v.atan() / FRAC_PI_4,
    })
}

pub(crate) fn vector_on_face(face: CubeFace, vector: UnitVec3) -> Option<(f64, f64)> {
    let point = vector.as_vec3();
    let (denominator, horizontal, vertical) = match face {
        CubeFace::PosX if point.x > 0.0 => (point.x, point.y, point.z),
        CubeFace::NegX if point.x < 0.0 => (-point.x, -point.y, point.z),
        CubeFace::PosY if point.y > 0.0 => (point.y, -point.x, point.z),
        CubeFace::NegY if point.y < 0.0 => (-point.y, point.x, point.z),
        CubeFace::PosZ if point.z > 0.0 => (point.z, point.x, point.y),
        CubeFace::NegZ if point.z < 0.0 => (-point.z, point.x, -point.y),
        _ => return None,
    };
    let face_s = (horizontal / denominator).atan() / FRAC_PI_4;
    let face_t = (vertical / denominator).atan() / FRAC_PI_4;
    (face_s.abs() <= 1.0 + 2.0e-14 && face_t.abs() <= 1.0 + 2.0e-14).then_some((face_s, face_t))
}

/// Locates the canonical cell containing a sphere vector.
///
/// # Errors
/// Returns [`SphereError::InvalidLevel`] above level 30 and propagates an
/// invalid vector error from [`vector_to_face`].
pub fn cell_at(vector: Vec3, level: u8) -> Result<CellId, SphereError> {
    if level > 30 {
        return Err(SphereError::InvalidLevel(level));
    }
    let coord = vector_to_face(vector)?;
    let count = 1_u32 << level;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = |value: f64| {
        let scaled = ((value + 1.0) * 0.5 * f64::from(count)).floor();
        scaled.clamp(0.0, f64::from(count - 1)) as u32
    };
    CellId::new(coord.face, level, index(coord.s), index(coord.t))
}
