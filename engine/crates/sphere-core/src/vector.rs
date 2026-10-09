use crate::SphereError;
use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x
            .mul_add(other.x, self.y.mul_add(other.y, self.z * other.z))
    }

    #[must_use]
    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y.mul_add(other.z, -self.z * other.y),
            self.z.mul_add(other.x, -self.x * other.z),
            self.x.mul_add(other.y, -self.y * other.x),
        )
    }

    #[must_use]
    pub fn norm(self) -> f64 {
        self.dot(self).sqrt()
    }

    #[must_use]
    pub fn normalized(self) -> Self {
        let norm = self.norm();
        Self::new(self.x / norm, self.y / norm, self.z / norm)
    }

    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y, -self.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec3", into = "Vec3")]
pub struct UnitVec3(Vec3);

impl UnitVec3 {
    /// Constructs a finite unit vector.
    ///
    /// # Errors
    /// Returns [`SphereError::InvalidVector`] for zero or non-finite input.
    pub fn new(value: Vec3) -> Result<Self, SphereError> {
        if !value.is_finite() {
            return Err(SphereError::InvalidVector);
        }
        let norm = value.norm();
        if !norm.is_finite() || norm == 0.0 {
            return Err(SphereError::InvalidVector);
        }
        Ok(Self(value * (1.0 / norm)))
    }

    #[must_use]
    pub const fn as_vec3(self) -> Vec3 {
        self.0
    }

    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.0.dot(other.0)
    }

    #[must_use]
    pub fn cross(self, other: Self) -> Vec3 {
        self.0.cross(other.0)
    }

    #[must_use]
    pub const fn negated(self) -> Self {
        Self(Vec3::new(-self.0.x, -self.0.y, -self.0.z))
    }
}

impl TryFrom<Vec3> for UnitVec3 {
    type Error = SphereError;
    fn try_from(value: Vec3) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<UnitVec3> for Vec3 {
    fn from(value: UnitVec3) -> Self {
        value.0
    }
}
