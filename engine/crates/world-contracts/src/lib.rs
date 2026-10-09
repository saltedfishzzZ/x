//! E0 共享契约：身份、状态、单位、资产头与规范摘要。

use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("non-finite floating-point values are not allowed in canonical JSON")]
    NonFiniteFloat,
    #[error("asset dependency list is not in canonical order")]
    NonCanonicalDependencies,
    #[error("unsupported asset-header contract version: {0}")]
    UnsupportedAssetHeaderVersion(u64),
    #[error("asset-header contract version must be an unsigned integer")]
    InvalidAssetHeaderVersion,
    #[error("invalid asset header: {0}")]
    InvalidAssetHeader(String),
}

pub const CURRENT_ASSET_HEADER_CONTRACT_VERSION: u32 = 2;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRefV2 {
    pub release_id: String,
    pub manifest_sha256: String,
}

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

string_id!(WorldIdentity);
string_id!(RunIdentity);
string_id!(ModelVersion);
string_id!(ParameterSetRef);
string_id!(AssetId);
string_id!(LayerReleaseId);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AssetClass {
    Authoritative,
    Index,
    Evidence,
    Presentation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CapabilityState {
    Available,
    Partial,
    NotAvailable,
    Unresolved,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QueryStatus {
    Available,
    OutsideCoverage,
    NotAvailable,
    Unresolved,
    Undefined,
    FailedDependency,
    CorruptAsset,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpatialCoverage {
    pub frame_id: String,
    pub face: Option<u8>,
    pub level: Option<u8>,
    pub tile_keys: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SupportScale {
    pub minimum_metres: f64,
    pub maximum_metres: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UnitDescriptor {
    pub symbol: String,
    pub datum: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DependencyRef {
    pub asset_id: AssetId,
    pub semantic_content_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetHeader {
    pub contract_version: u32,
    pub asset_id: AssetId,
    pub asset_class: AssetClass,
    pub schema_id: String,
    pub schema_version: u32,
    pub producer_model_id: String,
    pub algorithm_version: String,
    pub world_identity: WorldIdentity,
    pub layer_release_id: LayerReleaseId,
    pub direct_dependencies: Vec<DependencyRef>,
    pub semantic_content_hash: String,
    pub encoded_byte_hash: String,
    pub spatial_coverage: SpatialCoverage,
    pub support_scale: Option<SupportScale>,
    pub units: Option<UnitDescriptor>,
    pub capability: CapabilityState,
    pub created_by_run_id: RunIdentity,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAssetHeader {
    pub header: AssetHeader,
    pub source_contract_version: u32,
    pub canonical_semantic_header: Vec<u8>,
}

/// Reads an asset header while preserving the byte semantics of legacy v0.
///
/// A missing `contract_version` identifies v0. It is migrated in memory to v1,
/// while `canonical_semantic_header` retains the pre-migration representation
/// with an empty semantic hash for content verification.
///
/// # Errors
/// Returns a serialization error for malformed headers and a dedicated version
/// error for unsupported or invalid contract versions.
pub fn decode_asset_header(bytes: &[u8]) -> Result<DecodedAssetHeader, ContractError> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    let object = value.as_object().ok_or_else(|| {
        serde_json::Error::io(std::io::Error::other("asset header is not an object"))
    })?;
    let source_contract_version = match object.get("contract_version") {
        None => 0,
        Some(version) => version
            .as_u64()
            .ok_or(ContractError::InvalidAssetHeaderVersion)?
            .try_into()
            .map_err(|_| ContractError::InvalidAssetHeaderVersion)?,
    };
    if source_contract_version > CURRENT_ASSET_HEADER_CONTRACT_VERSION {
        return Err(ContractError::UnsupportedAssetHeaderVersion(u64::from(
            source_contract_version,
        )));
    }

    let mut semantic_value = value.clone();
    semantic_value
        .as_object_mut()
        .ok_or_else(|| {
            serde_json::Error::io(std::io::Error::other("asset header is not an object"))
        })?
        .insert(
            "semantic_content_hash".to_owned(),
            serde_json::Value::String(String::new()),
        );
    let canonical_semantic_header = canonical_json(&semantic_value)?;

    if source_contract_version == 0 {
        value
            .as_object_mut()
            .ok_or_else(|| {
                serde_json::Error::io(std::io::Error::other("asset header is not an object"))
            })?
            .insert(
                "contract_version".to_owned(),
                serde_json::Value::from(1_u32),
            );
    }
    let header: AssetHeader = serde_json::from_value(value)?;
    header.validate()?;
    Ok(DecodedAssetHeader {
        header,
        source_contract_version,
        canonical_semantic_header,
    })
}

impl AssetHeader {
    /// Validates canonical dependency ordering.
    ///
    /// # Errors
    /// Returns [`ContractError::NonCanonicalDependencies`] when dependencies
    /// are not ordered by stable asset identity.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self
            .direct_dependencies
            .windows(2)
            .any(|pair| pair[0].asset_id >= pair[1].asset_id)
        {
            return Err(ContractError::NonCanonicalDependencies);
        }
        if self.contract_version < 2 {
            return Ok(());
        }
        let identities = [
            self.asset_id.0.as_str(),
            self.schema_id.as_str(),
            self.producer_model_id.as_str(),
            self.algorithm_version.as_str(),
            self.world_identity.0.as_str(),
            self.layer_release_id.0.as_str(),
            self.created_by_run_id.0.as_str(),
        ];
        if identities.into_iter().any(|value| !safe_contract_id(value))
            || !sha256_digest(&self.semantic_content_hash)
            || !sha256_digest(&self.encoded_byte_hash)
            || self.direct_dependencies.iter().any(|dependency| {
                !safe_contract_id(&dependency.asset_id.0)
                    || !sha256_digest(&dependency.semantic_content_hash)
            })
        {
            return Err(ContractError::InvalidAssetHeader(
                "identity or digest is malformed".to_owned(),
            ));
        }
        if self.spatial_coverage.frame_id.is_empty()
            || self.spatial_coverage.face.is_some_and(|face| face > 5)
            || self.spatial_coverage.level.is_some_and(|level| level > 30)
            || self
                .spatial_coverage
                .tile_keys
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(ContractError::InvalidAssetHeader(
                "spatial coverage is invalid".to_owned(),
            ));
        }
        if let Some(scale) = &self.support_scale
            && (!scale.minimum_metres.is_finite()
                || !scale.maximum_metres.is_finite()
                || scale.minimum_metres < 0.0
                || scale.minimum_metres > scale.maximum_metres)
        {
            return Err(ContractError::InvalidAssetHeader(
                "support scale is invalid".to_owned(),
            ));
        }
        if self
            .units
            .as_ref()
            .is_some_and(|units| units.symbol.is_empty())
        {
            return Err(ContractError::InvalidAssetHeader(
                "unit symbol is empty".to_owned(),
            ));
        }
        Ok(())
    }
}

fn safe_contract_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
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

/// Deterministic `E0-CANONICAL-JSON-v1` encoding.
///
/// Object keys are recursively sorted, arrays retain source order, strings use
/// `serde_json` escaping, negative floating-point zero is normalized to `0`,
/// other finite numbers use its shortest round-trip spelling, and no
/// insignificant whitespace is emitted. Scientific missing states are
/// explicit enums, so non-finite floating values are rejected.
///
/// # Errors
/// Returns [`ContractError::Serialization`] when a value cannot be represented
/// in the E0 JSON subset.
pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ContractError> {
    value
        .serialize(FiniteFloatValidator)
        .map_err(|_| ContractError::NonFiniteFloat)?;
    let value = serde_json::to_value(value)?;
    let mut output = Vec::new();
    write_canonical_value(&value, &mut output)?;
    Ok(output)
}

#[derive(Debug)]
struct NonFiniteFloat;

impl std::fmt::Display for NonFiniteFloat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("non-finite floating-point value")
    }
}

impl std::error::Error for NonFiniteFloat {}

impl serde::ser::Error for NonFiniteFloat {
    fn custom<T: std::fmt::Display>(_message: T) -> Self {
        Self
    }
}

struct FiniteFloatValidator;

struct FiniteCompound;

macro_rules! valid_scalar {
    ($($method:ident($type:ty)),+ $(,)?) => {
        $(
            fn $method(self, _value: $type) -> Result<Self::Ok, Self::Error> {
                Ok(())
            }
        )+
    };
}

impl Serializer for FiniteFloatValidator {
    type Ok = ();
    type Error = NonFiniteFloat;
    type SerializeSeq = FiniteCompound;
    type SerializeTuple = FiniteCompound;
    type SerializeTupleStruct = FiniteCompound;
    type SerializeTupleVariant = FiniteCompound;
    type SerializeMap = FiniteCompound;
    type SerializeStruct = FiniteCompound;
    type SerializeStructVariant = FiniteCompound;

    valid_scalar!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_char(char),
        serialize_str(&str),
        serialize_bytes(&[u8]),
    );

    fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(NonFiniteFloat)
        }
    }

    fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(NonFiniteFloat)
        }
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_seq(self, _length: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(FiniteCompound)
    }

    fn serialize_tuple(self, _length: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(FiniteCompound)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(FiniteCompound)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(FiniteCompound)
    }

    fn serialize_map(self, _length: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(FiniteCompound)
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(FiniteCompound)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _length: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(FiniteCompound)
    }
}

macro_rules! compound_value {
    ($trait_name:ident, $method:ident) => {
        impl $trait_name for FiniteCompound {
            type Ok = ();
            type Error = NonFiniteFloat;

            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
                value.serialize(FiniteFloatValidator)
            }

            fn end(self) -> Result<Self::Ok, Self::Error> {
                Ok(())
            }
        }
    };
}

compound_value!(SerializeSeq, serialize_element);
compound_value!(SerializeTuple, serialize_element);
compound_value!(SerializeTupleStruct, serialize_field);

impl SerializeTupleVariant for FiniteCompound {
    type Ok = ();
    type Error = NonFiniteFloat;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteFloatValidator)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl SerializeMap for FiniteCompound {
    type Ok = ();
    type Error = NonFiniteFloat;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        key.serialize(FiniteFloatValidator)
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FiniteFloatValidator)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl SerializeStruct for FiniteCompound {
    type Ok = ();
    type Error = NonFiniteFloat;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(FiniteFloatValidator)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl SerializeStructVariant for FiniteCompound {
    type Ok = ();
    type Error = NonFiniteFloat;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(FiniteFloatValidator)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

fn write_canonical_value(
    value: &serde_json::Value,
    output: &mut Vec<u8>,
) -> Result<(), serde_json::Error> {
    match value {
        serde_json::Value::Null => output.extend_from_slice(b"null"),
        serde_json::Value::Bool(value) => {
            output.extend_from_slice(if *value { b"true" } else { b"false" });
        }
        serde_json::Value::Number(number) => {
            if number.as_f64().is_some_and(|value| value == 0.0) {
                output.push(b'0');
            } else {
                output.extend_from_slice(number.to_string().as_bytes());
            }
        }
        serde_json::Value::String(string) => {
            output.extend_from_slice(serde_json::to_string(string)?.as_bytes());
        }
        serde_json::Value::Array(values) => {
            output.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                write_canonical_value(item, output)?;
            }
            output.push(b']');
        }
        serde_json::Value::Object(values) => {
            output.push(b'{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                output.extend_from_slice(serde_json::to_string(key)?.as_bytes());
                output.push(b':');
                write_canonical_value(item, output)?;
            }
            output.push(b'}');
        }
    }
    Ok(())
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn canonical_map_order_is_stable() {
        let mut first = BTreeMap::new();
        first.insert("z", 1_u32);
        first.insert("a", 2_u32);
        let bytes = canonical_json(&first).unwrap();
        assert_eq!(bytes, br#"{"a":2,"z":1}"#);
        assert_eq!(sha256_hex(&bytes).len(), 64);
    }

    #[test]
    fn canonical_json_recursively_ignores_hash_map_insertion_order() {
        let mut first = HashMap::new();
        first.insert("z", vec![2_u32, 1]);
        first.insert("a", vec![4_u32, 3]);
        let mut second = HashMap::new();
        second.insert("a", vec![4_u32, 3]);
        second.insert("z", vec![2_u32, 1]);
        assert_eq!(
            canonical_json(&first).unwrap(),
            canonical_json(&second).unwrap()
        );
        assert_eq!(canonical_json(&first).unwrap(), br#"{"a":[4,3],"z":[2,1]}"#);
    }

    #[test]
    fn canonical_json_rejects_non_finite_numbers() {
        assert!(canonical_json(&f64::NAN).is_err());
        assert!(canonical_json(&f64::INFINITY).is_err());
        assert!(canonical_json(&vec![Some(1.0_f64), Some(f64::NEG_INFINITY)]).is_err());
    }

    #[test]
    fn canonical_json_normalizes_negative_zero() {
        assert_eq!(canonical_json(&-0.0_f64).unwrap(), b"0");
        assert_eq!(canonical_json(&0.0_f64).unwrap(), b"0");
    }

    #[test]
    fn canonical_json_preserves_unicode_scalar_sequence() {
        let composed = canonical_json(&"\u{e9}").unwrap();
        let decomposed = canonical_json(&"e\u{301}").unwrap();
        assert_ne!(composed, decomposed);
        assert_eq!(composed, "\"\u{e9}\"".as_bytes());
        assert_eq!(decomposed, "\"e\u{301}\"".as_bytes());
    }

    #[test]
    fn canonical_json_extreme_exponents_are_stable() {
        assert_eq!(canonical_json(&1.0e300_f64).unwrap(), b"1e+300");
        assert_eq!(canonical_json(&1.0e-300_f64).unwrap(), b"1e-300");
    }

    #[test]
    fn dependencies_must_be_sorted() {
        let header = AssetHeader {
            contract_version: CURRENT_ASSET_HEADER_CONTRACT_VERSION,
            asset_id: "asset".into(),
            asset_class: AssetClass::Authoritative,
            schema_id: "test".into(),
            schema_version: 1,
            producer_model_id: "model".into(),
            algorithm_version: "1".into(),
            world_identity: "world".into(),
            layer_release_id: "layer".into(),
            direct_dependencies: vec![
                DependencyRef {
                    asset_id: "z".into(),
                    semantic_content_hash: "1".into(),
                },
                DependencyRef {
                    asset_id: "a".into(),
                    semantic_content_hash: "2".into(),
                },
            ],
            semantic_content_hash: String::new(),
            encoded_byte_hash: String::new(),
            spatial_coverage: SpatialCoverage {
                frame_id: "frame".into(),
                face: None,
                level: None,
                tile_keys: vec![],
            },
            support_scale: None,
            units: None,
            capability: CapabilityState::Available,
            created_by_run_id: "run".into(),
            attributes: BTreeMap::new(),
        };
        assert!(matches!(
            header.validate(),
            Err(ContractError::NonCanonicalDependencies)
        ));
    }

    #[test]
    fn v2_asset_header_rejects_invalid_hash_scale_and_coverage() {
        let mut header = AssetHeader {
            contract_version: CURRENT_ASSET_HEADER_CONTRACT_VERSION,
            asset_id: "asset".into(),
            asset_class: AssetClass::Authoritative,
            schema_id: "test".into(),
            schema_version: 1,
            producer_model_id: "model".into(),
            algorithm_version: "v2".into(),
            world_identity: "world".into(),
            layer_release_id: "layer".into(),
            direct_dependencies: vec![],
            semantic_content_hash: "a".repeat(64),
            encoded_byte_hash: "b".repeat(64),
            spatial_coverage: SpatialCoverage {
                frame_id: "frame".into(),
                face: Some(0),
                level: Some(8),
                tile_keys: vec![1, 2],
            },
            support_scale: Some(SupportScale {
                minimum_metres: 1.0,
                maximum_metres: 2.0,
            }),
            units: Some(UnitDescriptor {
                symbol: "m".into(),
                datum: None,
            }),
            capability: CapabilityState::Available,
            created_by_run_id: "run".into(),
            attributes: BTreeMap::new(),
        };
        header.validate().unwrap();
        header.encoded_byte_hash = "bad".into();
        assert!(matches!(
            header.validate(),
            Err(ContractError::InvalidAssetHeader(_))
        ));
        header.encoded_byte_hash = "b".repeat(64);
        header.support_scale.as_mut().unwrap().minimum_metres = 3.0;
        assert!(matches!(
            header.validate(),
            Err(ContractError::InvalidAssetHeader(_))
        ));
        header.support_scale = None;
        header.spatial_coverage.face = Some(6);
        assert!(matches!(
            header.validate(),
            Err(ContractError::InvalidAssetHeader(_))
        ));
    }

    #[test]
    fn legacy_asset_header_migrates_without_changing_semantic_identity() {
        let decoded = decode_asset_header(include_bytes!(
            "../../../fixtures/contracts/asset-header-v0.json"
        ))
        .unwrap();
        assert_eq!(decoded.source_contract_version, 0);
        assert_eq!(decoded.header.contract_version, 1);
        assert_eq!(decoded.header.semantic_content_hash, "legacy-semantic-hash");
        let semantic: serde_json::Value =
            serde_json::from_slice(&decoded.canonical_semantic_header).unwrap();
        assert!(semantic.get("contract_version").is_none());
        assert_eq!(semantic["semantic_content_hash"], "");
    }

    #[test]
    fn current_asset_header_reads_without_migration() {
        let decoded = decode_asset_header(include_bytes!(
            "../../../fixtures/contracts/asset-header-v1.json"
        ))
        .unwrap();
        assert_eq!(decoded.source_contract_version, 1);
        assert_eq!(decoded.header.contract_version, 1);
    }

    #[test]
    fn future_asset_header_version_is_rejected() {
        let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../fixtures/contracts/asset-header-v1.json"
        ))
        .unwrap();
        value["contract_version"] = serde_json::Value::from(3_u32);
        let error = decode_asset_header(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(matches!(
            error,
            ContractError::UnsupportedAssetHeaderVersion(3)
        ));
    }
}
