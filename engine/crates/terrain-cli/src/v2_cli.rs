use clap::{Subcommand, ValueEnum};
use release_store::{CandidateState, ReleaseRoot, ReleaseStore, SafeStoreId};
use sphere_core::{CellId, CubeFace, PatchPlan, PatchRequest, SpatialReference};
use sphere_release::v2::{
    SpatialReleaseManifestV2, open_v2 as open_spatial, publish_v2 as publish_spatial,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use tectonics_core::FixedPlateInput;
use tectonics_core::v2::{build_fixed_skeleton, build_kinematics, build_natural_skeleton};
use tectonics_release::v2::{
    TectonicManifestV2, open_v2 as open_tectonics, publish_v2 as publish_tectonics,
};
use terrain_input::RunProfile;
use terrain_input::v2::{
    FileSystemReleaseResolverV2, GENERATION_SPEC_FILE_V2, M1AuthoritativeRequestV2,
    M1InputReleaseV2, ValidationStatusV2, WORLD_INTENT_FILE_V2, open_v2 as open_m1,
    publish_v2 as publish_m1,
};
use world_contracts::{ReleaseRefV2, canonical_json, sha256_hex};
use world_spec::v2::{GenerationSpecV2, WorldIntentSpecV2, WorldKindV2};
use world_spec::{PlateCountPreference, TectonicActivityPreference};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PlateCountV2Arg {
    Fewer,
    Standard,
    More,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ActivityV2Arg {
    Lower,
    Standard,
    Higher,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ProfileV2Arg {
    FixedTest,
    Diagnostic,
    StandardGlobal,
    RegionalRefinement,
}

#[derive(Debug, Subcommand)]
pub enum SphereV2Command {
    Build {
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        release_id: String,
        #[arg(long, default_value = "spatial-v2-candidate")]
        candidate_id: String,
        #[arg(long = "core-parent")]
        core_parents: Vec<CellId>,
    },
    Inspect {
        release_dir: PathBuf,
    },
    Validate {
        release_dir: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum WorldSpecV2Command {
    Build {
        #[arg(long)]
        seed: String,
        #[arg(long)]
        spatial_release_dir: PathBuf,
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long, value_enum, default_value_t = PlateCountV2Arg::Standard)]
        plate_count: PlateCountV2Arg,
        #[arg(long, value_enum, default_value_t = ActivityV2Arg::Standard)]
        activity: ActivityV2Arg,
        #[arg(long)]
        fixed_fixture_id: Option<String>,
        #[arg(long)]
        fixed_fixture_sha256: Option<String>,
        #[arg(long, default_value = "world-intent-v2-candidate")]
        intent_candidate_id: String,
        #[arg(long, default_value = "generation-spec-v2-candidate")]
        generation_candidate_id: String,
    },
    Inspect {
        #[arg(long)]
        world_intent_file: PathBuf,
        #[arg(long)]
        generation_spec_file: Option<PathBuf>,
    },
    Validate {
        #[arg(long)]
        world_intent_file: PathBuf,
        #[arg(long)]
        generation_spec_file: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TectonicsV2Command {
    Build {
        #[arg(long)]
        world_intent_file: PathBuf,
        #[arg(long)]
        generation_spec_file: PathBuf,
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        release_id: String,
        #[arg(long)]
        fixed_fixture_file: Option<PathBuf>,
        #[arg(long, default_value = "tectonics-v2-candidate")]
        candidate_id: String,
    },
    Inspect {
        release_dir: PathBuf,
    },
    Validate {
        release_dir: PathBuf,
    },
    Query {
        #[arg(long)]
        release_dir: PathBuf,
        #[arg(long)]
        cell: CellId,
    },
    Conformance {
        #[arg(long)]
        world_intent_file: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum TerrainInputV2Command {
    Build {
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        world_intent_release_dir: PathBuf,
        #[arg(long)]
        generation_spec_release_dir: PathBuf,
        #[arg(long)]
        spatial_release_dir: PathBuf,
        #[arg(long)]
        tectonic_release_dir: PathBuf,
        #[arg(long, value_enum, default_value_t = ProfileV2Arg::Diagnostic)]
        profile: ProfileV2Arg,
        #[arg(long, default_value = "diagnostic/default")]
        output_namespace: String,
        #[arg(long, default_value = "m1-v2-candidate")]
        candidate_id: String,
    },
    Inspect {
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        release_dir: PathBuf,
    },
    Validate {
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        release_dir: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum StoreCommand {
    Inspect {
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        candidate_id: String,
    },
    Recover {
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        candidate_id: String,
    },
    Validate {
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        candidate_id: String,
    },
}

pub fn run_sphere(command: SphereV2Command) -> Result<(), Box<dyn Error>> {
    match command {
        SphereV2Command::Build {
            store_root,
            release_id,
            candidate_id,
            mut core_parents,
        } => {
            if core_parents.is_empty() {
                core_parents.push(CellId::new(CubeFace::PosX, 8, 255, 128)?);
            }
            let request = PatchRequest {
                patch_id: "prototype-focus-v2".to_owned(),
                core_parents,
                target_level: 10,
                halo_parent_rings: 1,
                cell_budget: 1_000_000,
            };
            let plan = PatchPlan::build(request.clone())?;
            let manifest = SpatialReleaseManifestV2::build(
                release_id,
                &SpatialReference::prototype()?,
                vec![(request, plan)],
                Vec::new(),
            )?;
            let published = publish_spatial(&store_root, &candidate_id, &manifest)?;
            println!(
                "{}",
                serde_json::json!({
                    "release_id": manifest.spatial_release_id,
                    "manifest_sha256": published.manifest_sha256,
                    "path": published.path,
                    "visibility": published.visibility,
                    "durability": published.durability,
                    "reused_existing": published.reused_existing
                })
            );
        }
        SphereV2Command::Inspect { release_dir } => {
            println!("{}", serde_json::to_string(&open_spatial(&release_dir)?)?);
        }
        SphereV2Command::Validate { release_dir } => {
            let manifest = open_spatial(&release_dir)?;
            println!(
                "{}",
                serde_json::json!({"status":"VALID","validation_level":"FULL","release_id":manifest.spatial_release_id})
            );
        }
    }
    Ok(())
}

pub fn run_world_spec(command: WorldSpecV2Command) -> Result<(), Box<dyn Error>> {
    match command {
        WorldSpecV2Command::Build {
            seed,
            spatial_release_dir,
            store_root,
            plate_count,
            activity,
            fixed_fixture_id,
            fixed_fixture_sha256,
            intent_candidate_id,
            generation_candidate_id,
        } => {
            let mut intent = match (fixed_fixture_id, fixed_fixture_sha256) {
                (None, None) => WorldIntentSpecV2::earthlike_natural(&seed)?,
                (Some(id), Some(hash)) => WorldIntentSpecV2::earthlike_fixed_test(&seed, id, hash)?,
                _ => return Err("fixed fixture id and SHA-256 must be supplied together".into()),
            };
            intent.plate_count_preference = plate_count.into();
            intent.tectonic_activity_preference = activity.into();
            intent.validate()?;
            let spatial = open_spatial(&spatial_release_dir)?;
            let spatial_ref =
                release_ref(&spatial_release_dir, &spatial.spatial_release_id, &spatial)?;
            let generation = GenerationSpecV2::new(
                &intent,
                spatial_ref,
                BTreeMap::from([
                    ("tectonics".to_owned(), "tectonics-v2".to_owned()),
                    ("terrain-input".to_owned(), "terrain-input-v2".to_owned()),
                ]),
            )?;
            let world_identity = intent.identity()?;
            let generation_identity = generation.identity(&intent)?;
            let intent_ref = publish_json(
                &store_root,
                &intent_candidate_id,
                &world_identity.world_id,
                WORLD_INTENT_FILE_V2,
                &intent,
            )?;
            let generation_ref = publish_json(
                &store_root,
                &generation_candidate_id,
                &generation_identity.generation_identity,
                GENERATION_SPEC_FILE_V2,
                &generation,
            )?;
            println!(
                "{}",
                serde_json::json!({
                    "world_intent": intent_ref,
                    "generation_spec": generation_ref,
                    "plate_count": intent.plate_count()?,
                    "durability": "DURABILITY_NOT_PROVEN"
                })
            );
        }
        WorldSpecV2Command::Inspect {
            world_intent_file,
            generation_spec_file,
        } => {
            let intent: WorldIntentSpecV2 = read_canonical(&world_intent_file)?;
            let generation = generation_spec_file
                .as_deref()
                .map(read_canonical::<GenerationSpecV2>)
                .transpose()?;
            println!(
                "{}",
                serde_json::json!({"intent":intent,"generation":generation})
            );
        }
        WorldSpecV2Command::Validate {
            world_intent_file,
            generation_spec_file,
        } => {
            let intent: WorldIntentSpecV2 = read_canonical(&world_intent_file)?;
            intent.validate()?;
            let generation_identity = if let Some(path) = generation_spec_file {
                let generation: GenerationSpecV2 = read_canonical(&path)?;
                Some(generation.identity(&intent)?.generation_identity)
            } else {
                None
            };
            println!(
                "{}",
                serde_json::json!({"status":"VALID","world_id":intent.identity()?.world_id,"generation_identity":generation_identity})
            );
        }
    }
    Ok(())
}

pub fn run_tectonics(command: TectonicsV2Command) -> Result<(), Box<dyn Error>> {
    match command {
        TectonicsV2Command::Build {
            world_intent_file,
            generation_spec_file,
            store_root,
            release_id,
            fixed_fixture_file,
            candidate_id,
        } => {
            let intent: WorldIntentSpecV2 = read_canonical(&world_intent_file)?;
            let generation: GenerationSpecV2 = read_canonical(&generation_spec_file)?;
            let identity = generation.identity(&intent)?;
            let skeleton = match (intent.world_kind, fixed_fixture_file) {
                (WorldKindV2::Natural, None) => build_natural_skeleton(&intent)?,
                (WorldKindV2::FixedTest, Some(path)) => {
                    let fixture: Vec<FixedPlateInput> = read_canonical(&path)?;
                    build_fixed_skeleton(&intent, &fixture)?
                }
                (WorldKindV2::Natural, Some(_)) => {
                    return Err("natural worlds cannot use a fixed fixture".into());
                }
                (WorldKindV2::FixedTest, None) => {
                    return Err("fixed-test worlds require --fixed-fixture-file".into());
                }
            };
            let kinematics = build_kinematics(&intent, &identity, &skeleton)?;
            let manifest = TectonicManifestV2::build(
                release_id,
                &identity,
                generation.spatial_release.clone(),
                skeleton,
                kinematics,
            )?;
            let published = publish_tectonics(&store_root, &candidate_id, &manifest)?;
            println!(
                "{}",
                serde_json::json!({
                    "release_id": manifest.tectonic_release_id,
                    "manifest_sha256": published.manifest_sha256,
                    "path": published.path,
                    "visibility": published.visibility,
                    "durability": published.durability,
                    "reused_existing": published.reused_existing
                })
            );
        }
        TectonicsV2Command::Inspect { release_dir } => {
            println!("{}", serde_json::to_string(&open_tectonics(&release_dir)?)?);
        }
        TectonicsV2Command::Validate { release_dir } => {
            let manifest = open_tectonics(&release_dir)?;
            println!(
                "{}",
                serde_json::json!({"status":"VALID","release_id":manifest.tectonic_release_id})
            );
        }
        TectonicsV2Command::Query { release_dir, cell } => {
            let manifest = open_tectonics(&release_dir)?;
            if cell.level() != manifest.skeleton.level {
                return Err("query cell must use the materialized tectonic level".into());
            }
            let count = 1_usize << cell.level();
            let index = usize::from(cell.face() as u8) * count * count
                + usize::try_from(cell.j())? * count
                + usize::try_from(cell.i())?;
            let owner = *manifest
                .skeleton
                .owners
                .get(index)
                .ok_or("owner index outside field")?;
            let role = manifest
                .skeleton
                .plate_roles
                .get(usize::from(owner))
                .ok_or("owner role is invalid")?;
            println!(
                "{}",
                serde_json::json!({"status":"AVAILABLE","cell":cell,"plate_role_id":role.plate_role_id,"release_id":manifest.tectonic_release_id})
            );
        }
        TectonicsV2Command::Conformance { world_intent_file } => {
            let intent: WorldIntentSpecV2 = read_canonical(&world_intent_file)?;
            let skeleton = build_natural_skeleton(&intent)?;
            println!(
                "{}",
                serde_json::json!({
                    "world_id": intent.identity()?.world_id,
                    "tectonic_skeleton_id": skeleton.tectonic_skeleton_id,
                    "owners_sha256": sha256_hex(&canonical_json(&skeleton.owners)?),
                    "plate_count": skeleton.plate_roles.len()
                })
            );
        }
    }
    Ok(())
}

pub fn run_terrain_input(command: TerrainInputV2Command) -> Result<(), Box<dyn Error>> {
    match command {
        TerrainInputV2Command::Build {
            store_root,
            world_intent_release_dir,
            generation_spec_release_dir,
            spatial_release_dir,
            tectonic_release_dir,
            profile,
            output_namespace,
            candidate_id,
        } => {
            let intent: WorldIntentSpecV2 =
                read_canonical(&world_intent_release_dir.join(WORLD_INTENT_FILE_V2))?;
            let generation: GenerationSpecV2 =
                read_canonical(&generation_spec_release_dir.join(GENERATION_SPEC_FILE_V2))?;
            let spatial = open_spatial(&spatial_release_dir)?;
            let tectonic = open_tectonics(&tectonic_release_dir)?;
            let intent_ref = release_ref(
                &world_intent_release_dir,
                &intent.identity()?.world_id,
                &intent,
            )?;
            let generation_ref = release_ref(
                &generation_spec_release_dir,
                &generation.identity(&intent)?.generation_identity,
                &generation,
            )?;
            let spatial_ref =
                release_ref(&spatial_release_dir, &spatial.spatial_release_id, &spatial)?;
            let tectonic_ref = release_ref(
                &tectonic_release_dir,
                &tectonic.tectonic_release_id,
                &tectonic,
            )?;
            let request = M1AuthoritativeRequestV2::new(
                intent_ref,
                generation_ref,
                spatial_ref,
                tectonic_ref,
                profile.into(),
                output_namespace,
                vec![
                    "main_plate_domains".to_owned(),
                    "plate_euler_motion".to_owned(),
                ],
            )?;
            let resolver = FileSystemReleaseResolverV2::new(&store_root);
            let release = M1InputReleaseV2::build(request, &resolver)?;
            let published = publish_m1(&store_root, &candidate_id, &release, &resolver)?;
            println!(
                "{}",
                serde_json::json!({
                    "release_id": release.m1_release_id,
                    "release_sha256": published.release_sha256,
                    "path": published.path,
                    "admission": release.claims.admission,
                    "visibility": published.visibility,
                    "durability": published.durability,
                    "reused_existing": published.reused_existing
                })
            );
        }
        TerrainInputV2Command::Inspect {
            store_root,
            release_dir,
        } => {
            let resolver = FileSystemReleaseResolverV2::new(store_root);
            println!(
                "{}",
                serde_json::to_string(&open_m1(&release_dir, &resolver)?)?
            );
        }
        TerrainInputV2Command::Validate {
            store_root,
            release_dir,
        } => {
            let resolver = FileSystemReleaseResolverV2::new(store_root);
            let release = open_m1(&release_dir, &resolver)?;
            let report = release.validate_full_admission(&resolver);
            if report.status != ValidationStatusV2::Valid {
                return Err("M1 full admission failed".into());
            }
            println!("{}", serde_json::to_string(&report)?);
        }
    }
    Ok(())
}

pub fn run_store(command: StoreCommand) -> Result<(), Box<dyn Error>> {
    let (store_root, candidate_id, mode) = match command {
        StoreCommand::Inspect {
            store_root,
            candidate_id,
        } => (store_root, candidate_id, "INSPECT"),
        StoreCommand::Recover {
            store_root,
            candidate_id,
        } => (store_root, candidate_id, "RECOVER"),
        StoreCommand::Validate {
            store_root,
            candidate_id,
        } => (store_root, candidate_id, "VALIDATE"),
    };
    let id = SafeStoreId::parse(&candidate_id)?;
    let store = ReleaseStore::new(&store_root)?;
    let candidate = store_root.join("candidates").join(id.as_str());
    let transaction = store.read_candidate(&candidate)?;
    let result = match (mode, transaction.state) {
        ("RECOVER", CandidateState::Prepared) => "INTERRUPTED_PREPARED_REQUIRES_CALLER_DECISION",
        ("RECOVER", CandidateState::Validated) => "SEALED_CANDIDATE_READY_FOR_EXPLICIT_COMMIT",
        ("VALIDATE", CandidateState::Prepared) => "PREPARED_NOT_SEALED",
        ("VALIDATE", CandidateState::Validated) => "VALIDATED_METADATA_ONLY",
        _ => "READABLE",
    };
    println!(
        "{}",
        serde_json::json!({"mode":mode,"result":result,"transaction":transaction,"durability":"DURABILITY_NOT_PROVEN"})
    );
    Ok(())
}

fn publish_json<T: serde::Serialize>(
    store_root: &Path,
    candidate_id: &str,
    release_id: &str,
    file_name: &str,
    value: &T,
) -> Result<ReleaseRefV2, Box<dyn Error>> {
    let store = ReleaseStore::new(store_root)?;
    let candidate = store.create_candidate_v2(candidate_id, release_id)?;
    let bytes = canonical_json(value)?;
    let hash = sha256_hex(&bytes);
    fs::write(candidate.join(file_name), bytes)?;
    store.mark_candidate_validated(&candidate)?;
    store.commit_v2(
        &candidate,
        &ReleaseRoot {
            release_id: release_id.to_owned(),
            asset_ids: Vec::new(),
        },
    )?;
    Ok(ReleaseRefV2 {
        release_id: release_id.to_owned(),
        manifest_sha256: hash,
    })
}

fn release_ref<T: serde::Serialize>(
    release_dir: &Path,
    expected_id: &str,
    value: &T,
) -> Result<ReleaseRefV2, Box<dyn Error>> {
    let root: ReleaseRoot = serde_json::from_slice(&fs::read(release_dir.join("release.json"))?)?;
    if root.release_id != expected_id {
        return Err("release root identity mismatch".into());
    }
    Ok(ReleaseRefV2 {
        release_id: expected_id.to_owned(),
        manifest_sha256: sha256_hex(&canonical_json(value)?),
    })
}

fn read_canonical<T: serde::de::DeserializeOwned + serde::Serialize>(
    path: &Path,
) -> Result<T, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let value: T = serde_json::from_slice(&bytes)?;
    if canonical_json(&value)? != bytes {
        return Err("input JSON is not canonical".into());
    }
    Ok(value)
}

impl From<PlateCountV2Arg> for PlateCountPreference {
    fn from(value: PlateCountV2Arg) -> Self {
        match value {
            PlateCountV2Arg::Fewer => Self::Fewer,
            PlateCountV2Arg::Standard => Self::Standard,
            PlateCountV2Arg::More => Self::More,
        }
    }
}

impl From<ActivityV2Arg> for TectonicActivityPreference {
    fn from(value: ActivityV2Arg) -> Self {
        match value {
            ActivityV2Arg::Lower => Self::Lower,
            ActivityV2Arg::Standard => Self::Standard,
            ActivityV2Arg::Higher => Self::Higher,
        }
    }
}

impl From<ProfileV2Arg> for RunProfile {
    fn from(value: ProfileV2Arg) -> Self {
        match value {
            ProfileV2Arg::FixedTest => Self::FixedTest,
            ProfileV2Arg::Diagnostic => Self::Diagnostic,
            ProfileV2Arg::StandardGlobal => Self::StandardGlobal,
            ProfileV2Arg::RegionalRefinement => Self::RegionalRefinement,
        }
    }
}
