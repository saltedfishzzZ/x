//! E2 世界规格、规范种子、生成身份与无状态命名随机协议。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use world_contracts::{canonical_json, sha256_hex};

pub mod v2;

pub const WORLD_SPEC_SCHEMA: &str = "world-spec-v1";
pub const SEED_FORMAT: &str = "seed-format-v1";
pub const RANDOM_PROTOCOL: &str = "named-random-sha256-v1";
pub const DEFAULT_RADIUS_METRES: u64 = 6_371_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PlateCountPreference {
    Fewer,
    #[default]
    Standard,
    More,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TectonicActivityPreference {
    Lower,
    #[default]
    Standard,
    Higher,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSpec {
    pub schema: String,
    pub normalized_seed: String,
    pub plate_count_preference: PlateCountPreference,
    pub tectonic_activity_preference: TectonicActivityPreference,
    pub radius_metres: u64,
    pub spatial_reference_id: String,
    pub tectonic_model_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorldIdentity {
    pub world_id: String,
    pub seed_key: String,
    pub generation_identity: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundedSample {
    pub value: u64,
    pub attempts: u32,
}

#[derive(Debug, Error)]
pub enum WorldSpecError {
    #[error("seed is empty after normalization")]
    EmptySeed,
    #[error("seed contains a control character")]
    SeedControlCharacter,
    #[error("seed contains more than 128 Unicode scalar values")]
    SeedTooLong,
    #[error("random bound must be positive")]
    ZeroBound,
    #[error("world specification is unsupported or noncanonical")]
    UnsupportedSpec,
    #[error("canonical world specification failed: {0}")]
    Contract(#[from] world_contracts::ContractError),
}

impl WorldSpec {
    /// Builds the default Earth-like E2 specification.
    ///
    /// # Errors
    /// Returns a seed-format error for empty, overlong, or control-bearing text.
    pub fn earthlike(seed: &str) -> Result<Self, WorldSpecError> {
        Ok(Self {
            schema: WORLD_SPEC_SCHEMA.to_owned(),
            normalized_seed: normalize_seed(seed)?,
            plate_count_preference: PlateCountPreference::Standard,
            tectonic_activity_preference: TectonicActivityPreference::Standard,
            radius_metres: DEFAULT_RADIUS_METRES,
            spatial_reference_id: "cubed-sphere-equiangular-v1".to_owned(),
            tectonic_model_version: "tectonics-prototype-v1".to_owned(),
        })
    }

    /// Computes all E2 identities from canonical content.
    ///
    /// # Errors
    /// Propagates canonical serialization failures.
    pub fn identity(&self) -> Result<WorldIdentity, WorldSpecError> {
        let spec = canonical_json(self)?;
        let world_id = format!("world-{}", &sha256_hex(&spec)[..24]);
        let seed_key = sha256_hex(&frame(&[
            SEED_FORMAT.as_bytes(),
            self.normalized_seed.as_bytes(),
        ]));
        let generation = frame(&[
            b"generation-identity-v1",
            world_id.as_bytes(),
            self.spatial_reference_id.as_bytes(),
            self.tectonic_model_version.as_bytes(),
            RANDOM_PROTOCOL.as_bytes(),
        ]);
        Ok(WorldIdentity {
            world_id,
            seed_key,
            generation_identity: format!("generation-{}", &sha256_hex(&generation)[..24]),
        })
    }

    /// Validates that a serialized specification is canonical and supported.
    ///
    /// # Errors
    /// Rejects unsupported schema, radius, model, frame, or seed text.
    pub fn validate(&self) -> Result<(), WorldSpecError> {
        if self.schema != WORLD_SPEC_SCHEMA
            || self.radius_metres != DEFAULT_RADIUS_METRES
            || self.spatial_reference_id != "cubed-sphere-equiangular-v1"
            || self.tectonic_model_version != "tectonics-prototype-v1"
            || normalize_seed(&self.normalized_seed)? != self.normalized_seed
        {
            return Err(WorldSpecError::UnsupportedSpec);
        }
        Ok(())
    }

    /// Returns the monotonic plate-count choice for this seed.
    ///
    /// # Errors
    /// Propagates identity or random protocol errors.
    pub fn plate_count(&self) -> Result<u8, WorldSpecError> {
        let random = NamedRandom::from_spec(self)?;
        let u = random.unit_interval("tectonics", "world", "plate-count-quantile", 0);
        let value = match self.plate_count_preference {
            PlateCountPreference::Fewer => 6 + bucket(u, 2),
            PlateCountPreference::Standard => 8 + bucket(u, 3),
            PlateCountPreference::More => 11 + bucket(u, 2),
        };
        Ok(value)
    }

    /// Returns the per-plate positive Euler rate in microdegrees per million years.
    ///
    /// # Errors
    /// Propagates identity failures.
    pub fn euler_rate(&self, plate_entity: &str) -> Result<u32, WorldSpecError> {
        let random = NamedRandom::from_spec(self)?;
        let u = random.unit_interval("tectonics", plate_entity, "euler-rate-quantile", 0);
        let (minimum, maximum) = match self.tectonic_activity_preference {
            TectonicActivityPreference::Lower => (50_000, 350_000),
            TectonicActivityPreference::Standard => (200_000, 700_000),
            TectonicActivityPreference::Higher => (500_000, 1_000_000),
        };
        Ok(scale_inclusive(u, minimum, maximum))
    }
}

/// Normalizes seed text using NFC, outer trimming, and internal whitespace collapse.
///
/// # Errors
/// Rejects control characters, empty text, and more than 128 scalar values.
pub fn normalize_seed(seed: &str) -> Result<String, WorldSpecError> {
    let nfc: String = seed.nfc().collect();
    if nfc.chars().any(char::is_control) {
        return Err(WorldSpecError::SeedControlCharacter);
    }
    let normalized = nfc.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = normalized.chars().count();
    if count == 0 {
        return Err(WorldSpecError::EmptySeed);
    }
    if count > 128 {
        return Err(WorldSpecError::SeedTooLong);
    }
    Ok(normalized)
}

#[derive(Clone, Debug)]
pub struct NamedRandom {
    seed_key: String,
}

impl NamedRandom {
    /// Creates a named sampler from a world specification.
    ///
    /// # Errors
    /// Propagates identity failures.
    pub fn from_spec(spec: &WorldSpec) -> Result<Self, WorldSpecError> {
        Ok(Self {
            seed_key: spec.identity()?.seed_key,
        })
    }

    /// Creates a named sampler from an already validated SHA-256 seed key.
    ///
    /// # Errors
    /// Rejects values that are not lowercase hexadecimal SHA-256 digests.
    pub fn from_seed_key(seed_key: &str) -> Result<Self, WorldSpecError> {
        if seed_key.len() != 64
            || !seed_key
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(WorldSpecError::UnsupportedSpec);
        }
        Ok(Self {
            seed_key: seed_key.to_owned(),
        })
    }

    #[must_use]
    pub fn sample_u64(
        &self,
        namespace: &str,
        entity_id: &str,
        property: &str,
        sample_index: u64,
    ) -> u64 {
        let index = sample_index.to_be_bytes();
        let bytes = frame(&[
            RANDOM_PROTOCOL.as_bytes(),
            self.seed_key.as_bytes(),
            namespace.as_bytes(),
            entity_id.as_bytes(),
            property.as_bytes(),
            &index,
        ]);
        let digest = Sha256::digest(bytes);
        let mut leading = [0_u8; 8];
        leading.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(leading)
    }

    #[must_use]
    pub fn unit_interval(
        &self,
        namespace: &str,
        entity_id: &str,
        property: &str,
        sample_index: u64,
    ) -> f64 {
        const DENOMINATOR: f64 = 9_007_199_254_740_992.0;
        let bits = self.sample_u64(namespace, entity_id, property, sample_index) >> 11;
        #[allow(clippy::cast_precision_loss)]
        let numerator = bits as f64;
        numerator / DENOMINATOR
    }

    /// Samples an unbiased integer below `bound` using rejection sampling.
    ///
    /// # Errors
    /// Returns [`WorldSpecError::ZeroBound`] for a zero bound.
    pub fn bounded(
        &self,
        namespace: &str,
        entity_id: &str,
        property: &str,
        sample_index: u64,
        bound: u64,
    ) -> Result<BoundedSample, WorldSpecError> {
        if bound == 0 {
            return Err(WorldSpecError::ZeroBound);
        }
        let threshold = bound.wrapping_neg() % bound;
        for attempt in 0_u32.. {
            let value = self.sample_u64(
                namespace,
                entity_id,
                property,
                sample_index.wrapping_add(u64::from(attempt)),
            );
            if value >= threshold {
                return Ok(BoundedSample {
                    value: value % bound,
                    attempts: attempt + 1,
                });
            }
        }
        unreachable!("an unbiased u64 sample must eventually be accepted")
    }
}

#[must_use]
pub fn frame(fields: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for field in fields {
        let length = u64::try_from(field.len()).unwrap_or(u64::MAX);
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.extend_from_slice(field);
    }
    bytes
}

fn bucket(unit: f64, count: u8) -> u8 {
    let scaled = unit * f64::from(count);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let value = scaled.floor() as u8;
    value.min(count - 1)
}

fn scale_inclusive(unit: f64, minimum: u32, maximum: u32) -> u32 {
    let width = u64::from(maximum - minimum) + 1;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    let offset = (unit * width as f64).floor() as u64;
    minimum + u32::try_from(offset.min(width - 1)).expect("range is u32")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_normalization_is_nfc_and_whitespace_stable() {
        assert_eq!(normalize_seed("  e\u{301}  world ").unwrap(), "é world");
        assert!(normalize_seed("line\nbreak").is_err());
        assert!(normalize_seed("   ").is_err());
    }

    #[test]
    fn identities_are_stable_and_case_sensitive() {
        let upper = WorldSpec::earthlike("Gaia").unwrap().identity().unwrap();
        let repeated = WorldSpec::earthlike("Gaia").unwrap().identity().unwrap();
        let lower = WorldSpec::earthlike("gaia").unwrap().identity().unwrap();
        assert_eq!(upper, repeated);
        assert_ne!(upper.world_id, lower.world_id);
        assert_ne!(upper.seed_key, lower.seed_key);
    }

    #[test]
    fn e2f01_and_e2f02_fixed_vectors_match() {
        let spec = WorldSpec::earthlike("e\u{301}  world").unwrap();
        let identity = spec.identity().unwrap();
        assert_eq!(spec.normalized_seed, "é world");
        assert_eq!(identity.world_id, "world-5b78f49088a044d83ec64ed5");
        assert_eq!(
            identity.seed_key,
            "89ad75f74c5f3875b9789bb6e4f203c8c695d66e9229ee3be2bf5c02c815e267"
        );

        let pv01 = WorldSpec::earthlike("PV01-E2-v1").unwrap();
        let random = NamedRandom::from_spec(&pv01).unwrap();
        assert_eq!(
            random.sample_u64("tectonics", "world", "plate-count-quantile", 0),
            14_770_547_165_756_481_792
        );
    }

    #[test]
    fn named_random_is_order_independent_and_isolated() {
        let spec = WorldSpec::earthlike("random-test").unwrap();
        let random = NamedRandom::from_spec(&spec).unwrap();
        let first = random.sample_u64("a", "entity", "property", 7);
        let unrelated = random.sample_u64("b", "entity", "property", 7);
        assert_eq!(first, random.sample_u64("a", "entity", "property", 7));
        assert_ne!(first, unrelated);
        let bounded = random.bounded("a", "entity", "bounded", 0, 7).unwrap();
        assert!(bounded.value < 7);
        assert!(bounded.attempts >= 1);
    }

    #[test]
    fn preferences_are_monotonic_for_one_seed() {
        let mut spec = WorldSpec::earthlike("preference-test").unwrap();
        spec.plate_count_preference = PlateCountPreference::Fewer;
        let fewer = spec.plate_count().unwrap();
        spec.plate_count_preference = PlateCountPreference::Standard;
        let standard = spec.plate_count().unwrap();
        spec.plate_count_preference = PlateCountPreference::More;
        let more = spec.plate_count().unwrap();
        assert!((6..=7).contains(&fewer));
        assert!((8..=10).contains(&standard));
        assert!((11..=12).contains(&more));
        assert!(fewer < standard && standard < more);
    }
}
