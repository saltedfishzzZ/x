//! E2.1 v2世界意图、生成规格与分层身份。

use crate::{
    DEFAULT_RADIUS_METRES, NamedRandom, PlateCountPreference, SEED_FORMAT,
    TectonicActivityPreference, WorldSpecError, frame, normalize_seed,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use world_contracts::{canonical_json, sha256_hex};

pub use world_contracts::ReleaseRefV2;

pub const WORLD_INTENT_SCHEMA_V2: &str = "world-intent-spec-v2";
pub const GENERATION_SPEC_SCHEMA_V2: &str = "generation-spec-v2";
pub const EARTHLIKE_PRESET_V1: &str = "earthlike-prototype-v1";
pub const RANDOM_PROTOCOL_V1: &str = "named-random-sha256-v1";
pub const NUMERIC_PROTOCOL_V2: &str = "deterministic-numeric-v2";
pub const PLATE_SITE_MODEL_V2: &str = "plate-sites-v2";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorldKindV2 {
    Natural,
    FixedTest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedTestDescriptorV2 {
    pub fixture_id: String,
    pub fixture_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldIntentSpecV2 {
    pub schema: String,
    pub normalized_seed: String,
    pub seed_format_version: String,
    pub preset: String,
    pub radius_metres: u64,
    pub plate_count_preference: PlateCountPreference,
    pub tectonic_activity_preference: TectonicActivityPreference,
    pub world_kind: WorldKindV2,
    pub fixed_test: Option<FixedTestDescriptorV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorldIntentIdentityV2 {
    pub world_id: String,
    pub seed_key: String,
    pub world_intent_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationSpecV2 {
    pub schema: String,
    pub world_id: String,
    pub world_intent_sha256: String,
    pub spatial_release: ReleaseRefV2,
    pub model_manifest: BTreeMap<String, String>,
    pub random_protocol_version: String,
    pub numeric_protocol_version: String,
    pub fixed_test_override: Option<FixedTestDescriptorV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GenerationIdentityV2 {
    pub world_id: String,
    pub generation_identity: String,
}

impl WorldIntentSpecV2 {
    /// Builds a natural Earth-like v2 world intent.
    ///
    /// # Errors
    /// Returns a seed normalization error for invalid text.
    pub fn earthlike_natural(seed: &str) -> Result<Self, WorldSpecError> {
        Ok(Self {
            schema: WORLD_INTENT_SCHEMA_V2.to_owned(),
            normalized_seed: normalize_seed(seed)?,
            seed_format_version: SEED_FORMAT.to_owned(),
            preset: EARTHLIKE_PRESET_V1.to_owned(),
            radius_metres: DEFAULT_RADIUS_METRES,
            plate_count_preference: PlateCountPreference::Standard,
            tectonic_activity_preference: TectonicActivityPreference::Standard,
            world_kind: WorldKindV2::Natural,
            fixed_test: None,
        })
    }

    /// Builds a fixed-test Earth-like v2 world intent.
    ///
    /// # Errors
    /// Rejects invalid seed text, fixture identity, or fixture digest.
    pub fn earthlike_fixed_test(
        seed: &str,
        fixture_id: impl Into<String>,
        fixture_sha256: impl Into<String>,
    ) -> Result<Self, WorldSpecError> {
        let intent = Self {
            schema: WORLD_INTENT_SCHEMA_V2.to_owned(),
            normalized_seed: normalize_seed(seed)?,
            seed_format_version: SEED_FORMAT.to_owned(),
            preset: EARTHLIKE_PRESET_V1.to_owned(),
            radius_metres: DEFAULT_RADIUS_METRES,
            plate_count_preference: PlateCountPreference::Standard,
            tectonic_activity_preference: TectonicActivityPreference::Standard,
            world_kind: WorldKindV2::FixedTest,
            fixed_test: Some(FixedTestDescriptorV2 {
                fixture_id: fixture_id.into(),
                fixture_sha256: fixture_sha256.into(),
            }),
        };
        intent.validate()?;
        Ok(intent)
    }

    /// Recomputes and validates the user-visible v2 world identity.
    ///
    /// # Errors
    /// Rejects unsupported fields or canonical serialization failures.
    pub fn identity(&self) -> Result<WorldIntentIdentityV2, WorldSpecError> {
        self.validate()?;
        let bytes = canonical_json(self)?;
        let digest = sha256_hex(&bytes);
        let seed_key = sha256_hex(&frame(&[
            SEED_FORMAT.as_bytes(),
            self.normalized_seed.as_bytes(),
        ]));
        Ok(WorldIntentIdentityV2 {
            world_id: format!("world-v2-{}", &digest[..24]),
            seed_key,
            world_intent_sha256: digest,
        })
    }

    /// Returns the deterministic plate count for this intent.
    ///
    /// # Errors
    /// Propagates v2 identity and random protocol errors.
    pub fn plate_count(&self) -> Result<u8, WorldSpecError> {
        let identity = self.identity()?;
        let random = NamedRandom::from_seed_key(&identity.seed_key)?;
        let count = match self.plate_count_preference {
            PlateCountPreference::Standard => 3,
            PlateCountPreference::Fewer | PlateCountPreference::More => 2,
        };
        let sample = random.bounded(
            "tectonics-skeleton-v2",
            "world",
            "plate-count-quantile",
            0,
            count,
        )?;
        let offset = u8::try_from(sample.value).map_err(|_| WorldSpecError::UnsupportedSpec)?;
        Ok(match self.plate_count_preference {
            PlateCountPreference::Fewer => 6 + offset,
            PlateCountPreference::Standard => 8 + offset,
            PlateCountPreference::More => 11 + offset,
        })
    }

    /// Checks the complete v2 world-intent invariant set.
    ///
    /// # Errors
    /// Rejects noncanonical seeds, unsupported constants, or inconsistent fixed-test state.
    pub fn validate(&self) -> Result<(), WorldSpecError> {
        let fixed_valid = match (&self.world_kind, &self.fixed_test) {
            (WorldKindV2::Natural, None) => true,
            (WorldKindV2::FixedTest, Some(fixed)) => {
                safe_id(&fixed.fixture_id) && sha256_digest(&fixed.fixture_sha256)
            }
            _ => false,
        };
        if self.schema != WORLD_INTENT_SCHEMA_V2
            || self.seed_format_version != SEED_FORMAT
            || self.preset != EARTHLIKE_PRESET_V1
            || self.radius_metres != DEFAULT_RADIUS_METRES
            || normalize_seed(&self.normalized_seed)? != self.normalized_seed
            || !fixed_valid
        {
            return Err(WorldSpecError::UnsupportedSpec);
        }
        Ok(())
    }
}

impl GenerationSpecV2 {
    /// Builds a v2 generation specification from a validated world intent.
    ///
    /// # Errors
    /// Rejects invalid release references, model versions, or world intent.
    pub fn new(
        intent: &WorldIntentSpecV2,
        spatial_release: ReleaseRefV2,
        model_manifest: BTreeMap<String, String>,
    ) -> Result<Self, WorldSpecError> {
        let identity = intent.identity()?;
        let spec = Self {
            schema: GENERATION_SPEC_SCHEMA_V2.to_owned(),
            world_id: identity.world_id,
            world_intent_sha256: identity.world_intent_sha256,
            spatial_release,
            model_manifest,
            random_protocol_version: RANDOM_PROTOCOL_V1.to_owned(),
            numeric_protocol_version: NUMERIC_PROTOCOL_V2.to_owned(),
            fixed_test_override: intent.fixed_test.clone(),
        };
        spec.validate(intent)?;
        Ok(spec)
    }

    /// Recomputes the v2 generation identity.
    ///
    /// # Errors
    /// Rejects a specification that does not match the supplied world intent.
    pub fn identity(
        &self,
        intent: &WorldIntentSpecV2,
    ) -> Result<GenerationIdentityV2, WorldSpecError> {
        self.validate(intent)?;
        let bytes = canonical_json(self)?;
        Ok(GenerationIdentityV2 {
            world_id: self.world_id.clone(),
            generation_identity: format!("generation-v2-{}", &sha256_hex(&bytes)[..24]),
        })
    }

    /// Checks exact world, release, model, random, and fixed-test references.
    ///
    /// # Errors
    /// Rejects mismatched intent identities and malformed references.
    pub fn validate(&self, intent: &WorldIntentSpecV2) -> Result<(), WorldSpecError> {
        let identity = intent.identity()?;
        if self.schema != GENERATION_SPEC_SCHEMA_V2
            || self.world_id != identity.world_id
            || self.world_intent_sha256 != identity.world_intent_sha256
            || !safe_id(&self.spatial_release.release_id)
            || !sha256_digest(&self.spatial_release.manifest_sha256)
            || self.model_manifest.is_empty()
            || self
                .model_manifest
                .iter()
                .any(|(model, version)| !safe_id(model) || !safe_id(version))
            || self.random_protocol_version != RANDOM_PROTOCOL_V1
            || self.numeric_protocol_version != NUMERIC_PROTOCOL_V2
            || self.fixed_test_override != intent.fixed_test
        {
            return Err(WorldSpecError::UnsupportedSpec);
        }
        Ok(())
    }
}

fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn sha256_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct IdentityFixture {
        fixture_id: String,
        schema: String,
        seed: String,
        fixed_fixture_id: String,
        fixed_fixture_sha256: String,
        expect_distinct_world_ids: bool,
    }

    fn models() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("tectonics".to_owned(), "tectonics-v2".to_owned()),
            ("terrain-input".to_owned(), "terrain-input-v2".to_owned()),
        ])
    }

    fn spatial() -> ReleaseRefV2 {
        ReleaseRefV2 {
            release_id: "spatial-v2".to_owned(),
            manifest_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn natural_and_fixed_test_worlds_have_distinct_identities() {
        let natural = WorldIntentSpecV2::earthlike_natural("same-seed").unwrap();
        let fixed = WorldIntentSpecV2::earthlike_fixed_test("same-seed", "PV01-v2", "b".repeat(64))
            .unwrap();
        let natural_id = natural.identity().unwrap();
        let fixed_id = fixed.identity().unwrap();
        assert_eq!(natural_id.seed_key, fixed_id.seed_key);
        assert_ne!(natural_id.world_id, fixed_id.world_id);
        assert_ne!(
            GenerationSpecV2::new(&natural, spatial(), models())
                .unwrap()
                .identity(&natural)
                .unwrap(),
            GenerationSpecV2::new(&fixed, spatial(), models())
                .unwrap()
                .identity(&fixed)
                .unwrap()
        );
    }

    #[test]
    fn identity_fixture_is_executable() {
        let fixture: IdentityFixture = serde_json::from_slice(include_bytes!(
            "../../../fixtures/e2.1/identity/E21I01-world-kinds.json"
        ))
        .unwrap();
        assert_eq!(fixture.fixture_id, "E21-I01");
        assert_eq!(fixture.schema, "e2.1-world-kinds-fixture-v1");
        assert!(fixture.expect_distinct_world_ids);
        let natural = WorldIntentSpecV2::earthlike_natural(&fixture.seed).unwrap();
        let fixed = WorldIntentSpecV2::earthlike_fixed_test(
            &fixture.seed,
            fixture.fixed_fixture_id,
            fixture.fixed_fixture_sha256,
        )
        .unwrap();
        assert_eq!(
            natural.identity().unwrap().seed_key,
            fixed.identity().unwrap().seed_key
        );
        assert_ne!(
            natural.identity().unwrap().world_id,
            fixed.identity().unwrap().world_id
        );
    }

    #[test]
    fn activity_changes_world_but_not_seed_or_plate_count() {
        let mut lower = WorldIntentSpecV2::earthlike_natural("activity-isolation").unwrap();
        lower.tectonic_activity_preference = TectonicActivityPreference::Lower;
        let mut higher = lower.clone();
        higher.tectonic_activity_preference = TectonicActivityPreference::Higher;
        let lower_id = lower.identity().unwrap();
        let higher_id = higher.identity().unwrap();
        assert_eq!(lower_id.seed_key, higher_id.seed_key);
        assert_ne!(lower_id.world_id, higher_id.world_id);
        assert_eq!(lower.plate_count().unwrap(), higher.plate_count().unwrap());
    }

    #[test]
    fn generation_identity_binds_exact_spatial_release_and_models() {
        let intent = WorldIntentSpecV2::earthlike_natural("generation-binding").unwrap();
        let first = GenerationSpecV2::new(&intent, spatial(), models())
            .unwrap()
            .identity(&intent)
            .unwrap();
        let mut changed_spatial = spatial();
        changed_spatial.manifest_sha256 = "c".repeat(64);
        let second = GenerationSpecV2::new(&intent, changed_spatial, models())
            .unwrap()
            .identity(&intent)
            .unwrap();
        let mut changed_models = models();
        changed_models.insert("tectonics".to_owned(), "tectonics-v2.1".to_owned());
        let third = GenerationSpecV2::new(&intent, spatial(), changed_models)
            .unwrap()
            .identity(&intent)
            .unwrap();
        assert_ne!(first, second);
        assert_ne!(first, third);
    }

    #[test]
    fn inconsistent_fixed_test_state_is_rejected() {
        let mut intent = WorldIntentSpecV2::earthlike_natural("invalid-fixed").unwrap();
        intent.fixed_test = Some(FixedTestDescriptorV2 {
            fixture_id: "PV01-v2".to_owned(),
            fixture_sha256: "d".repeat(64),
        });
        assert!(intent.validate().is_err());
    }
}
