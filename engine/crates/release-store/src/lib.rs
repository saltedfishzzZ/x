//! TS05：不可变字段块、双摘要与原子发布实验。

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use world_contracts::{AssetHeader, AssetId, canonical_json, decode_asset_header, sha256_hex};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("contract failed: {0}")]
    Contract(#[from] world_contracts::ContractError),
    #[error("JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("asset byte hash mismatch")]
    ByteHashMismatch,
    #[error("asset semantic hash mismatch")]
    SemanticHashMismatch,
    #[error("field byte length is invalid")]
    InvalidFieldLength,
    #[error("release already exists")]
    ReleaseExists,
    #[error("candidate already exists")]
    CandidateExists,
    #[error("unsafe store identity: {0}")]
    UnsafeId(String),
    #[error("candidate transaction is invalid: {0}")]
    InvalidTransaction(String),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SafeStoreId(String);

impl SafeStoreId {
    /// Parses the shared path-safe store identity format.
    ///
    /// # Errors
    /// Rejects path syntax, dot paths, non-ASCII text and Windows device names.
    pub fn parse(value: &str) -> Result<Self, StoreError> {
        let valid_chars = !value.is_empty()
            && value.len() <= 128
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        let stem = value
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0');
        if !valid_chars || value == "." || value == ".." || reserved {
            return Err(StoreError::UnsafeId(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandidateState {
    Prepared,
    Validated,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateTransaction {
    pub schema: String,
    pub candidate_id: SafeStoreId,
    pub release_id: SafeStoreId,
    pub state: CandidateState,
    pub validated_content_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VisibilityGuarantee {
    AtomicallyVisible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DurabilityGuarantee {
    DurabilityNotProven,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitResultV2 {
    pub path: PathBuf,
    pub visibility: VisibilityGuarantee,
    pub durability: DurabilityGuarantee,
    pub reused_existing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReleaseRoot {
    pub release_id: String,
    pub asset_ids: Vec<AssetId>,
}

#[derive(Clone, Debug)]
pub struct ReleaseStore {
    root: PathBuf,
}

impl ReleaseStore {
    /// Opens or creates a release store root.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] when required directories cannot be created.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let store = Self { root: root.into() };
        fs::create_dir_all(store.root.join("candidates"))?;
        fs::create_dir_all(store.root.join("releases"))?;
        Ok(store)
    }

    /// Creates a clean, non-published candidate directory.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] when an existing candidate cannot be removed
    /// or the new directory cannot be created.
    pub fn create_candidate(&self, candidate_id: &str) -> Result<PathBuf, StoreError> {
        let candidate_id = SafeStoreId::parse(candidate_id)?;
        let path = self.root.join("candidates").join(candidate_id.as_str());
        if path.exists() {
            return Err(StoreError::CandidateExists);
        }
        fs::create_dir_all(path.join("assets"))?;
        Ok(path)
    }

    /// Creates a recoverable v2 candidate transaction without replacing an
    /// existing directory.
    ///
    /// # Errors
    /// Rejects unsafe IDs, duplicate candidates, and I/O failures.
    pub fn create_candidate_v2(
        &self,
        candidate_id: &str,
        release_id: &str,
    ) -> Result<PathBuf, StoreError> {
        let candidate_id = SafeStoreId::parse(candidate_id)?;
        let release_id = SafeStoreId::parse(release_id)?;
        let path = self.root.join("candidates").join(candidate_id.as_str());
        if path.exists() {
            return Err(StoreError::CandidateExists);
        }
        fs::create_dir(&path)?;
        fs::create_dir(path.join("assets"))?;
        write_transaction(
            &path,
            &CandidateTransaction {
                schema: "release-candidate-transaction-v2".to_owned(),
                candidate_id,
                release_id,
                state: CandidateState::Prepared,
                validated_content_sha256: None,
            },
        )?;
        Ok(path)
    }

    /// Seals the current candidate bytes for commit.
    ///
    /// # Errors
    /// Rejects unknown or already-sealed candidates and I/O failures.
    pub fn mark_candidate_validated(&self, candidate: &Path) -> Result<String, StoreError> {
        let mut transaction = self.read_candidate(candidate)?;
        if transaction.state != CandidateState::Prepared {
            return Err(StoreError::InvalidTransaction(
                "candidate is not PREPARED".to_owned(),
            ));
        }
        let digest = directory_content_hash(candidate)?;
        transaction.state = CandidateState::Validated;
        transaction.validated_content_sha256 = Some(digest.clone());
        write_transaction(candidate, &transaction)?;
        Ok(digest)
    }

    /// Reads an interrupted candidate without changing or deleting it.
    ///
    /// # Errors
    /// Rejects paths outside this store and malformed transaction data.
    pub fn read_candidate(&self, candidate: &Path) -> Result<CandidateTransaction, StoreError> {
        self.validate_candidate_path(candidate)?;
        let bytes = fs::read(candidate.join("transaction.json"))?;
        let transaction: CandidateTransaction = serde_json::from_slice(&bytes)?;
        if canonical_json(&transaction)? != bytes
            || transaction.schema != "release-candidate-transaction-v2"
            || candidate.file_name().and_then(|name| name.to_str())
                != Some(transaction.candidate_id.as_str())
        {
            return Err(StoreError::InvalidTransaction(
                "transaction identity or encoding mismatch".to_owned(),
            ));
        }
        Ok(transaction)
    }

    /// Writes one canonical little-endian binary64 field and its asset header.
    ///
    /// # Errors
    /// Returns a contract error for invalid metadata and an I/O error when the
    /// asset cannot be persisted.
    pub fn write_field(
        &self,
        candidate: &Path,
        mut header: AssetHeader,
        values: &[f64],
    ) -> Result<AssetHeader, StoreError> {
        self.validate_candidate_path(candidate)?;
        let data = encode_f64(values);
        header.encoded_byte_hash = sha256_hex(&data);
        header.semantic_content_hash.clear();
        let mut semantic = canonical_json(&header)?;
        semantic.extend_from_slice(&data);
        header.semantic_content_hash = sha256_hex(&semantic);
        header.validate()?;
        SafeStoreId::parse(&header.asset_id.0)?;

        let asset_dir = candidate.join("assets").join(&header.asset_id.0);
        fs::create_dir_all(&asset_dir)?;
        fs::write(asset_dir.join("header.json"), canonical_json(&header)?)?;
        fs::write(asset_dir.join("field.f64le"), &data)?;
        Ok(header)
    }

    /// Verifies both hashes and decodes a field asset.
    ///
    /// # Errors
    /// Returns an I/O or decoding error, or a dedicated hash mismatch when
    /// either metadata or field bytes were changed.
    pub fn verify_field(&self, asset_dir: &Path) -> Result<(AssetHeader, Vec<f64>), StoreError> {
        let header_bytes = fs::read(asset_dir.join("header.json"))?;
        let decoded_header = decode_asset_header(&header_bytes)?;
        let mut header = decoded_header.header;
        let data = fs::read(asset_dir.join("field.f64le"))?;
        if sha256_hex(&data) != header.encoded_byte_hash {
            return Err(StoreError::ByteHashMismatch);
        }
        let expected_semantic = header.semantic_content_hash.clone();
        let mut semantic = decoded_header.canonical_semantic_header;
        semantic.extend_from_slice(&data);
        if sha256_hex(&semantic) != expected_semantic {
            return Err(StoreError::SemanticHashMismatch);
        }
        header.semantic_content_hash = expected_semantic;
        Ok((header, decode_f64(&data)?))
    }

    /// Atomically renames a complete candidate into the releases namespace.
    ///
    /// # Errors
    /// Returns [`StoreError::ReleaseExists`] for an immutable identity already
    /// present, or an I/O/serialization error before the rename completes.
    pub fn commit(&self, candidate: &Path, release: &ReleaseRoot) -> Result<PathBuf, StoreError> {
        self.validate_candidate_path(candidate)?;
        let release_id = SafeStoreId::parse(&release.release_id)?;
        for asset in &release.asset_ids {
            SafeStoreId::parse(&asset.0)?;
        }
        let target = self.root.join("releases").join(release_id.as_str());
        if target.exists() {
            return Err(StoreError::ReleaseExists);
        }
        fs::write(candidate.join("release.json"), canonical_json(release)?)?;
        fs::rename(candidate, &target)?;
        Ok(target)
    }

    /// Commits a sealed v2 candidate with explicit visibility and durability semantics.
    ///
    /// # Errors
    /// Rejects unvalidated, modified, conflicting, or path-unsafe candidates.
    pub fn commit_v2(
        &self,
        candidate: &Path,
        release: &ReleaseRoot,
    ) -> Result<CommitResultV2, StoreError> {
        self.validate_candidate_path(candidate)?;
        let transaction = self.read_candidate(candidate)?;
        let release_id = SafeStoreId::parse(&release.release_id)?;
        if transaction.release_id != release_id || transaction.state != CandidateState::Validated {
            return Err(StoreError::InvalidTransaction(
                "candidate is not validated for this release".to_owned(),
            ));
        }
        let expected = transaction
            .validated_content_sha256
            .as_deref()
            .ok_or_else(|| {
                StoreError::InvalidTransaction("validated digest is missing".to_owned())
            })?;
        if directory_content_hash(candidate)? != expected {
            return Err(StoreError::InvalidTransaction(
                "candidate changed after validation".to_owned(),
            ));
        }
        let target = self.root.join("releases").join(release_id.as_str());
        if target.exists() {
            let existing: ReleaseRoot =
                serde_json::from_slice(&fs::read(target.join("release.json"))?)?;
            let existing_transaction: CandidateTransaction =
                serde_json::from_slice(&fs::read(target.join("transaction.json"))?)?;
            if existing == *release
                && existing_transaction.validated_content_sha256.as_deref() == Some(expected)
            {
                return Ok(CommitResultV2 {
                    path: target,
                    visibility: VisibilityGuarantee::AtomicallyVisible,
                    durability: DurabilityGuarantee::DurabilityNotProven,
                    reused_existing: true,
                });
            }
            return Err(StoreError::ReleaseExists);
        }
        fs::write(candidate.join("release.json"), canonical_json(release)?)?;
        fs::rename(candidate, &target)?;
        Ok(CommitResultV2 {
            path: target,
            visibility: VisibilityGuarantee::AtomicallyVisible,
            durability: DurabilityGuarantee::DurabilityNotProven,
            reused_existing: false,
        })
    }

    fn validate_candidate_path(&self, candidate: &Path) -> Result<(), StoreError> {
        let parent = candidate
            .parent()
            .ok_or_else(|| StoreError::InvalidTransaction("candidate has no parent".to_owned()))?;
        let name = candidate
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                StoreError::InvalidTransaction("candidate name is invalid".to_owned())
            })?;
        SafeStoreId::parse(name)?;
        if parent != self.root.join("candidates")
            || !fs::symlink_metadata(candidate)?.file_type().is_dir()
        {
            return Err(StoreError::InvalidTransaction(
                "candidate is outside the store namespace or is a symlink".to_owned(),
            ));
        }
        Ok(())
    }
}

fn write_transaction(path: &Path, transaction: &CandidateTransaction) -> Result<(), StoreError> {
    fs::write(path.join("transaction.json"), canonical_json(transaction)?)?;
    Ok(())
}

fn directory_content_hash(path: &Path) -> Result<String, StoreError> {
    let mut files = Vec::new();
    collect_files(path, path, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut framed = Vec::new();
    for (relative, bytes) in files {
        if relative == "transaction.json" || relative == "release.json" {
            continue;
        }
        framed.extend_from_slice(&(relative.len() as u64).to_be_bytes());
        framed.extend_from_slice(relative.as_bytes());
        framed.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        framed.extend_from_slice(&bytes);
    }
    Ok(sha256_hex(&framed))
}

fn collect_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), StoreError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(root, &path, output)?;
        } else if entry.file_type()?.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| StoreError::InvalidTransaction("invalid candidate path".to_owned()))?
                .to_string_lossy()
                .replace('\\', "/");
            output.push((relative, fs::read(path)?));
        }
    }
    Ok(())
}

#[must_use]
pub fn encode_f64(values: &[f64]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 8);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decodes canonical little-endian binary64 values.
///
/// # Errors
/// Returns [`StoreError::InvalidFieldLength`] when the byte length is not a
/// multiple of eight.
pub fn decode_f64(bytes: &[u8]) -> Result<Vec<f64>, StoreError> {
    if !bytes.len().is_multiple_of(8) {
        return Err(StoreError::InvalidFieldLength);
    }
    Ok(bytes
        .chunks_exact(8)
        .map(|chunk| {
            let mut value = [0_u8; 8];
            value.copy_from_slice(chunk);
            f64::from_le_bytes(value)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};
    use world_contracts::{
        AssetClass, CapabilityState, LayerReleaseId, RunIdentity, SpatialCoverage, WorldIdentity,
    };

    fn temp_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("terrain-{name}-{}-{nonce}", std::process::id()))
    }

    fn header() -> AssetHeader {
        AssetHeader {
            contract_version: world_contracts::CURRENT_ASSET_HEADER_CONTRACT_VERSION,
            asset_id: AssetId("field-1".into()),
            asset_class: AssetClass::Authoritative,
            schema_id: "prototype.field".into(),
            schema_version: 1,
            producer_model_id: "ts05".into(),
            algorithm_version: "0.1".into(),
            world_identity: WorldIdentity("world".into()),
            layer_release_id: LayerReleaseId("layer".into()),
            direct_dependencies: vec![],
            semantic_content_hash: String::new(),
            encoded_byte_hash: String::new(),
            spatial_coverage: SpatialCoverage {
                frame_id: "frame".into(),
                face: Some(0),
                level: Some(0),
                tile_keys: vec![0],
            },
            support_scale: None,
            units: None,
            capability: CapabilityState::Available,
            created_by_run_id: RunIdentity("run".into()),
            attributes: BTreeMap::new(),
        }
    }

    #[test]
    fn field_round_trip_and_atomic_commit() {
        let root = temp_path("round-trip");
        let store = ReleaseStore::new(&root).unwrap();
        let candidate = store.create_candidate("candidate").unwrap();
        let written = store
            .write_field(&candidate, header(), &[1.0, -2.5, 3.25])
            .unwrap();
        let (read, values) = store
            .verify_field(&candidate.join("assets/field-1"))
            .unwrap();
        assert_eq!(written, read);
        assert_eq!(values, vec![1.0, -2.5, 3.25]);
        let target = store
            .commit(
                &candidate,
                &ReleaseRoot {
                    release_id: "release-1".into(),
                    asset_ids: vec![AssetId("field-1".into())],
                },
            )
            .unwrap();
        assert!(target.join("release.json").exists());
        assert!(!candidate.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn byte_tampering_is_detected() {
        let root = temp_path("tamper");
        let store = ReleaseStore::new(&root).unwrap();
        let candidate = store.create_candidate("candidate").unwrap();
        store.write_field(&candidate, header(), &[1.0]).unwrap();
        let asset_dir = candidate.join("assets/field-1");
        fs::write(asset_dir.join("field.f64le"), 2.0_f64.to_le_bytes()).unwrap();
        assert!(matches!(
            store.verify_field(&asset_dir),
            Err(StoreError::ByteHashMismatch)
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn interrupted_candidate_is_not_a_release() {
        let root = temp_path("interrupt");
        let store = ReleaseStore::new(&root).unwrap();
        let candidate = store.create_candidate("candidate").unwrap();
        store.write_field(&candidate, header(), &[1.0]).unwrap();
        assert!(!root.join("releases/candidate").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lossless_compression_does_not_change_semantic_bytes() {
        let canonical = encode_f64(&[0.0, -12.5, 9_001.25]);
        let compressed = zstd::stream::encode_all(canonical.as_slice(), 3).unwrap();
        let decoded = zstd::stream::decode_all(compressed.as_slice()).unwrap();
        assert_eq!(decoded, canonical);
        assert_eq!(sha256_hex(&decoded), sha256_hex(&canonical));
    }

    #[test]
    fn legacy_header_verifies_with_pre_migration_semantics() {
        let root = temp_path("legacy");
        let store = ReleaseStore::new(&root).unwrap();
        let candidate = store.create_candidate("candidate").unwrap();
        let asset_dir = candidate.join("assets/legacy-field");
        fs::create_dir_all(&asset_dir).unwrap();

        let data = encode_f64(&[1.5, -2.0]);
        let mut legacy: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../fixtures/contracts/asset-header-v0.json"
        ))
        .unwrap();
        legacy["encoded_byte_hash"] = serde_json::Value::String(sha256_hex(&data));
        legacy["semantic_content_hash"] = serde_json::Value::String(String::new());
        let mut semantic = canonical_json(&legacy).unwrap();
        semantic.extend_from_slice(&data);
        legacy["semantic_content_hash"] = serde_json::Value::String(sha256_hex(&semantic));

        fs::write(
            asset_dir.join("header.json"),
            canonical_json(&legacy).unwrap(),
        )
        .unwrap();
        fs::write(asset_dir.join("field.f64le"), data).unwrap();
        let (migrated, values) = store.verify_field(&asset_dir).unwrap();
        assert_eq!(migrated.contract_version, 1);
        assert_eq!(values, vec![1.5, -2.0]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn safe_store_ids_reject_paths_and_windows_devices() {
        for value in [
            "",
            ".",
            "..",
            "../escape",
            "a/b",
            "a\\b",
            "/root",
            "CON",
            "con.txt",
            "NUL",
            "COM1",
            "lpt9.log",
            "é",
        ] {
            assert!(SafeStoreId::parse(value).is_err(), "{value}");
        }
        for value in ["release-1", "world.v2", "A_b-9", "COM10"] {
            assert_eq!(SafeStoreId::parse(value).unwrap().as_str(), value);
        }
    }

    #[derive(Deserialize)]
    struct SafeStoreIdFixture {
        fixture_id: String,
        schema: String,
        accepted: Vec<String>,
        rejected: Vec<String>,
    }

    #[test]
    fn safe_store_id_fixture_is_executable() {
        let fixture: SafeStoreIdFixture = serde_json::from_slice(include_bytes!(
            "../../../fixtures/e2.1/storage/E21R01-safe-store-ids.json"
        ))
        .unwrap();
        assert_eq!(fixture.fixture_id, "E21-R01");
        assert_eq!(fixture.schema, "e2.1-safe-store-id-fixture-v1");
        for value in fixture.accepted {
            assert_eq!(SafeStoreId::parse(&value).unwrap().as_str(), value);
        }
        for value in fixture.rejected {
            assert!(SafeStoreId::parse(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn candidates_are_never_silently_replaced() {
        let root = temp_path("candidate-conflict");
        let store = ReleaseStore::new(&root).unwrap();
        let first = store.create_candidate("same").unwrap();
        fs::write(first.join("sentinel"), b"keep").unwrap();
        assert!(matches!(
            store.create_candidate("same"),
            Err(StoreError::CandidateExists)
        ));
        assert_eq!(fs::read(first.join("sentinel")).unwrap(), b"keep");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn v2_transaction_detects_interruption_mutation_and_idempotence() {
        let root = temp_path("transaction-v2");
        let store = ReleaseStore::new(&root).unwrap();
        let interrupted = store
            .create_candidate_v2("interrupted", "release-interrupted")
            .unwrap();
        assert_eq!(
            store.read_candidate(&interrupted).unwrap().state,
            CandidateState::Prepared
        );

        let candidate = store
            .create_candidate_v2("candidate-v2", "release-v2")
            .unwrap();
        fs::write(candidate.join("payload.bin"), b"stable").unwrap();
        store.mark_candidate_validated(&candidate).unwrap();
        fs::write(candidate.join("payload.bin"), b"changed").unwrap();
        assert!(matches!(
            store.commit_v2(
                &candidate,
                &ReleaseRoot {
                    release_id: "release-v2".to_owned(),
                    asset_ids: Vec::new(),
                }
            ),
            Err(StoreError::InvalidTransaction(_))
        ));

        fs::write(candidate.join("payload.bin"), b"stable").unwrap();
        let release = ReleaseRoot {
            release_id: "release-v2".to_owned(),
            asset_ids: Vec::new(),
        };
        let committed = store.commit_v2(&candidate, &release).unwrap();
        assert_eq!(committed.visibility, VisibilityGuarantee::AtomicallyVisible);
        assert_eq!(
            committed.durability,
            DurabilityGuarantee::DurabilityNotProven
        );
        assert!(!committed.reused_existing);

        let duplicate = store
            .create_candidate_v2("candidate-copy", "release-v2")
            .unwrap();
        fs::write(duplicate.join("payload.bin"), b"stable").unwrap();
        store.mark_candidate_validated(&duplicate).unwrap();
        let reused = store.commit_v2(&duplicate, &release).unwrap();
        assert!(reused.reused_existing);
        assert!(duplicate.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
