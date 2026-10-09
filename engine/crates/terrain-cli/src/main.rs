use clap::{Parser, Subcommand, ValueEnum};
use release_store::ReleaseStore;
use sphere_core::{
    CellId, CubeFace, PatchPlan, PatchRequest, SpatialReference, cube_rotations,
    unit_sphere_area_error,
};
use sphere_release::{SpatialReleaseManifest, open as open_spatial, publish as publish_spatial};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use tectonics_core::{
    ConstructionMode, TectonicWorld, build as build_tectonics, build_fixed, pv01_fixed_input,
};
use tectonics_release::{
    TectonicManifest, TectonicProvenance, open as open_tectonics, publish as publish_tectonics,
};
use terrain_input::{
    M1InputRelease, RunProfile, TerrainMaterialInputBundle, TerrainMaterialRunPlan,
    open as open_terrain_input, publish as publish_terrain_input,
};
use world_contracts::{canonical_json, sha256_hex};
use world_spec::{PlateCountPreference, TectonicActivityPreference, WorldSpec};

mod v2_cli;
use v2_cli::{
    SphereV2Command, StoreCommand, TectonicsV2Command, TerrainInputV2Command, WorldSpecV2Command,
};

#[derive(Debug, Parser)]
#[command(
    name = "terrain-cli",
    version,
    about = "类地行星引擎核心 T0/E1/E2 无界面入口"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 输出当前原型能力；尚未实现的自然能力保持明确不可用。
    Inspect,
    /// 输出 TS01 本机可复算的球面诊断指标。
    Diagnostics,
    /// 验证一个 TS05 字段资产目录。
    Validate { asset_dir: PathBuf },
    /// 输出跨平台逐字节比较所需的 E0 一致性证据。
    Conformance,
    /// 构建、检查、验证和查询 E1 球面空间发布。
    Sphere {
        #[command(subcommand)]
        command: SphereCommand,
    },
    /// 构建、检查和验证E2世界规格。
    WorldSpec {
        #[command(subcommand)]
        command: WorldSpecCommand,
    },
    /// 构建、检查、验证和查询E2构造发布。
    Tectonics {
        #[command(subcommand)]
        command: TectonicsCommand,
    },
    /// 构建、检查和验证E2 M1输入发布。
    TerrainInput {
        #[command(subcommand)]
        command: TerrainInputCommand,
    },
    /// 构建、检查和完整验证E2.1空间发布。
    SphereV2 {
        #[command(subcommand)]
        command: SphereV2Command,
    },
    /// 发布、检查和验证E2.1世界意图与生成规格。
    WorldSpecV2 {
        #[command(subcommand)]
        command: WorldSpecV2Command,
    },
    /// 构建、检查、验证和查询E2.1构造发布。
    TectonicsV2 {
        #[command(subcommand)]
        command: TectonicsV2Command,
    },
    /// 构建、检查和完整验证E2.1 M1输入发布。
    TerrainInputV2 {
        #[command(subcommand)]
        command: TerrainInputV2Command,
    },
    /// 检查E2.1候选事务和只读恢复状态。
    Store {
        #[command(subcommand)]
        command: StoreCommand,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum PlateCountArg {
    Fewer,
    Standard,
    More,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ActivityArg {
    Lower,
    Standard,
    Higher,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProvenanceArg {
    FixedTest,
    RandomPrototype,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum RunProfileArg {
    FixedTest,
    Diagnostic,
    StandardGlobal,
    RegionalRefinement,
}

#[derive(Debug, Subcommand)]
enum SphereCommand {
    /// 构建并原子发布一个 E1 空间清单。
    Build {
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        release_id: String,
        #[arg(long, default_value = "candidate")]
        candidate_id: String,
        #[arg(long = "core-parent")]
        core_parents: Vec<CellId>,
    },
    /// 检查内置空间规范，或读取一个已发布清单。
    Inspect {
        #[arg(long)]
        release_dir: Option<PathBuf>,
    },
    /// 完整验证一个已发布清单及其摘要。
    Validate { release_dir: PathBuf },
    /// 查询规范单元的中心、面积、边长和邻接距离。
    Query {
        #[arg(long)]
        cell: CellId,
        #[arg(long, default_value_t = 6_371_000.0)]
        radius_metres: f64,
    },
}

#[derive(Debug, Subcommand)]
enum WorldSpecCommand {
    /// 生成规范世界规格，并可写入文件。
    Build {
        #[arg(long)]
        seed: String,
        #[arg(long, value_enum, default_value_t = PlateCountArg::Standard)]
        plate_count: PlateCountArg,
        #[arg(long, value_enum, default_value_t = ActivityArg::Standard)]
        activity: ActivityArg,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// 读取世界规格并输出身份。
    Inspect { file: PathBuf },
    /// 验证世界规格。
    Validate { file: PathBuf },
}

#[derive(Debug, Subcommand)]
enum TectonicsCommand {
    /// 构建并原子发布一个E2 L8构造世界。
    Build {
        #[arg(long)]
        seed: String,
        #[arg(long)]
        spatial_release_dir: PathBuf,
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        release_id: String,
        #[arg(long, default_value = "e2-candidate")]
        candidate_id: String,
        #[arg(long, value_enum, default_value_t = ProvenanceArg::RandomPrototype)]
        provenance: ProvenanceArg,
    },
    /// 输出构造发布摘要。
    Inspect { release_dir: PathBuf },
    /// 完整验证构造发布。
    Validate { release_dir: PathBuf },
    /// 查询一个物化L8单元的板块owner。
    Query {
        #[arg(long)]
        release_dir: PathBuf,
        #[arg(long)]
        cell: CellId,
    },
    /// 输出与调度和存储无关的E2离散一致性证据。
    Conformance {
        #[arg(long)]
        seed: String,
        #[arg(long, value_enum, default_value_t = ProvenanceArg::RandomPrototype)]
        provenance: ProvenanceArg,
    },
}

#[derive(Debug, Subcommand)]
enum TerrainInputCommand {
    /// 从E1空间发布和E2构造发布建立M1输入发布。
    Build {
        #[arg(long)]
        seed: String,
        #[arg(long)]
        spatial_release_dir: PathBuf,
        #[arg(long)]
        tectonic_release_dir: PathBuf,
        #[arg(long)]
        store_root: PathBuf,
        #[arg(long)]
        release_id: String,
        #[arg(long, default_value = "m1-candidate")]
        candidate_id: String,
        #[arg(long, value_enum, default_value_t = RunProfileArg::Diagnostic)]
        profile: RunProfileArg,
        #[arg(long, default_value = "diagnostic/default")]
        output_namespace: String,
    },
    /// 输出M1输入发布。
    Inspect { release_dir: PathBuf },
    /// 完整验证M1输入发布。
    Validate { release_dir: PathBuf },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("command failed: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Inspect => {
            println!("{{\"profile\":\"T0_E0\",\"terrain_generation\":\"NOT_AVAILABLE\"}}");
        }
        Command::Diagnostics => {
            let area_error = unit_sphere_area_error(8).expect("L8 is a valid prototype level");
            println!(
                "{{\"cube_rotations\":{},\"l8_relative_area_error\":{area_error:.17e}}}",
                cube_rotations().len()
            );
        }
        Command::Validate { asset_dir } => {
            let root = asset_dir
                .parent()
                .and_then(|path| path.parent())
                .unwrap_or(&asset_dir);
            match ReleaseStore::new(root).and_then(|store| store.verify_field(&asset_dir)) {
                Ok((header, values)) => println!(
                    "{{\"asset_id\":{},\"status\":\"VALID\",\"value_count\":{}}}",
                    serde_json::to_string(&header.asset_id.0)?,
                    values.len()
                ),
                Err(error) => return Err(error.into()),
            }
        }
        Command::Conformance => {
            let mut nested = BTreeMap::new();
            nested.insert("z", serde_json::json!([3, 2, 1]));
            nested.insert("a", serde_json::json!(-0.0));
            let fixture = serde_json::json!({
                "contract": "E0-CANONICAL-JSON-v1",
                "extreme": 1.0e300,
                "nested": nested,
                "unicode": ["é", "e\u{301}"]
            });
            let canonical = canonical_json(&fixture).expect("fixed conformance fixture is valid");
            println!(
                "{}",
                serde_json::json!({
                    "canonical_json": String::from_utf8(canonical.clone())?,
                    "canonical_json_sha256": sha256_hex(&canonical),
                    "canonical_json_version": "E0-CANONICAL-JSON-v1"
                })
            );
        }
        Command::Sphere { command } => run_sphere(command)?,
        Command::WorldSpec { command } => run_world_spec(command)?,
        Command::Tectonics { command } => run_tectonics(command)?,
        Command::TerrainInput { command } => run_terrain_input(command)?,
        Command::SphereV2 { command } => v2_cli::run_sphere(command)?,
        Command::WorldSpecV2 { command } => v2_cli::run_world_spec(command)?,
        Command::TectonicsV2 { command } => v2_cli::run_tectonics(command)?,
        Command::TerrainInputV2 { command } => v2_cli::run_terrain_input(command)?,
        Command::Store { command } => v2_cli::run_store(command)?,
    }
    Ok(())
}

fn run_sphere(command: SphereCommand) -> Result<(), Box<dyn Error>> {
    match command {
        SphereCommand::Build {
            store_root,
            release_id,
            candidate_id,
            mut core_parents,
        } => {
            if core_parents.is_empty() {
                core_parents.push(CellId::new(CubeFace::PosX, 8, 120, 130)?);
            }
            let patch = PatchPlan::build(PatchRequest {
                patch_id: "prototype-focus".into(),
                core_parents,
                target_level: 10,
                halo_parent_rings: 1,
                cell_budget: 1_000_000,
            })?;
            let manifest = SpatialReleaseManifest::e1(
                release_id,
                SpatialReference::prototype()?,
                vec![patch],
            )?;
            let published = publish_spatial(&store_root, &candidate_id, &manifest)?;
            println!("{}", serde_json::to_string(&published)?);
        }
        SphereCommand::Inspect { release_dir } => {
            let manifest = if let Some(path) = release_dir {
                open_spatial(&path)?
            } else {
                SpatialReleaseManifest::e1("preview", SpatialReference::prototype()?, Vec::new())?
            };
            println!("{}", serde_json::to_string(&manifest)?);
        }
        SphereCommand::Validate { release_dir } => {
            let manifest = open_spatial(&release_dir)?;
            println!(
                "{}",
                serde_json::json!({
                    "spatial_release_id": manifest.spatial_release_id,
                    "status": "VALID"
                })
            );
        }
        SphereCommand::Query {
            cell,
            radius_metres,
        } => {
            let reference = SpatialReference::new(radius_metres, 8, 10)?;
            println!("{}", serde_json::to_string(&cell.geometry(&reference)?)?);
        }
    }
    Ok(())
}

fn run_world_spec(command: WorldSpecCommand) -> Result<(), Box<dyn Error>> {
    match command {
        WorldSpecCommand::Build {
            seed,
            plate_count,
            activity,
            output,
        } => {
            let mut spec = WorldSpec::earthlike(&seed)?;
            spec.plate_count_preference = plate_count.into();
            spec.tectonic_activity_preference = activity.into();
            spec.validate()?;
            let bytes = canonical_json(&spec)?;
            if let Some(path) = output {
                fs::write(path, &bytes)?;
            }
            println!(
                "{}",
                serde_json::json!({
                    "identity": spec.identity()?,
                    "plate_count": spec.plate_count()?,
                    "spec_sha256": sha256_hex(&bytes)
                })
            );
        }
        WorldSpecCommand::Inspect { file } => {
            let spec = read_world_spec(&file)?;
            println!(
                "{}",
                serde_json::json!({
                    "identity": spec.identity()?,
                    "plate_count": spec.plate_count()?,
                    "spec": spec
                })
            );
        }
        WorldSpecCommand::Validate { file } => {
            let spec = read_world_spec(&file)?;
            println!(
                "{}",
                serde_json::json!({"status":"VALID", "world_id":spec.identity()?.world_id})
            );
        }
    }
    Ok(())
}

fn run_tectonics(command: TectonicsCommand) -> Result<(), Box<dyn Error>> {
    match command {
        TectonicsCommand::Build {
            seed,
            spatial_release_dir,
            store_root,
            release_id,
            candidate_id,
            provenance,
        } => {
            let spec = WorldSpec::earthlike(&seed)?;
            let spatial = open_spatial(&spatial_release_dir)?;
            let spatial_hash = sha256_hex(&canonical_json(&spatial)?);
            let world = match provenance {
                ProvenanceArg::FixedTest => build_fixed(&spec, &pv01_fixed_input()?)?,
                ProvenanceArg::RandomPrototype => build_tectonics(&spec)?,
            };
            let manifest = TectonicManifest::from_world(
                release_id,
                &world,
                spatial.spatial_release_id,
                spatial_hash,
            );
            let published =
                publish_tectonics(&store_root, &candidate_id, &manifest, &world.owners)?;
            println!("{}", serde_json::to_string(&published)?);
        }
        TectonicsCommand::Inspect { release_dir } => {
            let opened = open_tectonics(&release_dir)?;
            println!(
                "{}",
                serde_json::json!({
                    "boundary_component_count": opened.manifest.boundary_components.len(),
                    "boundary_edge_count": opened.manifest.boundary_edges.len(),
                    "generation_attempt": opened.manifest.generation_attempt,
                    "manifest_sha256": opened.manifest_sha256,
                    "plate_count": opened.manifest.plates.len(),
                    "schema": opened.manifest.schema,
                    "tectonic_release_id": opened.manifest.tectonic_release_id,
                    "world_id": opened.manifest.world_id
                })
            );
        }
        TectonicsCommand::Validate { release_dir } => {
            let opened = open_tectonics(&release_dir)?;
            println!(
                "{}",
                serde_json::json!({
                    "status":"VALID",
                    "tectonic_release_id":opened.manifest.tectonic_release_id
                })
            );
        }
        TectonicsCommand::Query { release_dir, cell } => {
            query_tectonic_owner(&release_dir, cell)?;
        }
        TectonicsCommand::Conformance { seed, provenance } => {
            run_tectonic_conformance(&seed, provenance)?;
        }
    }
    Ok(())
}

fn query_tectonic_owner(release_dir: &Path, cell: CellId) -> Result<(), Box<dyn Error>> {
    let opened = open_tectonics(release_dir)?;
    let tectonic_release_id = opened.manifest.tectonic_release_id.clone();
    let world = TectonicWorld {
        world_id: opened.manifest.world_id,
        generation_identity: opened.manifest.generation_identity,
        level: opened.manifest.level,
        generation_attempt: opened.manifest.generation_attempt,
        construction_mode: match opened.manifest.provenance {
            TectonicProvenance::FixedTest => ConstructionMode::FixedTest,
            TectonicProvenance::RandomPrototype => ConstructionMode::RandomPrototype,
        },
        plates: opened.manifest.plates,
        owners: opened.owners,
        boundary_edges: opened.manifest.boundary_edges,
        boundary_components: opened.manifest.boundary_components,
        kinematic_samples: opened.manifest.kinematic_samples,
    };
    let plate = world.owner_of_cell(cell)?;
    println!(
        "{}",
        serde_json::json!({
            "knowledge_status":"AVAILABLE",
            "plate":plate,
            "query_method":"MATERIALIZED_L8_OWNER",
            "release_id":tectonic_release_id,
            "resolution_or_scale":"L8",
            "source_object_ids":[cell],
            "unit":null
        })
    );
    Ok(())
}

fn run_tectonic_conformance(seed: &str, provenance: ProvenanceArg) -> Result<(), Box<dyn Error>> {
    let spec = WorldSpec::earthlike(seed)?;
    let world = match provenance {
        ProvenanceArg::FixedTest => build_fixed(&spec, &pv01_fixed_input()?)?,
        ProvenanceArg::RandomPrototype => build_tectonics(&spec)?,
    };
    let qualifications = world.kinematic_samples.iter().fold(
        BTreeMap::<String, usize>::new(),
        |mut counts, sample| {
            *counts
                .entry(format!("{:?}", sample.qualification).to_uppercase())
                .or_default() += 1;
            counts
        },
    );
    println!(
        "{}",
        serde_json::json!({
            "boundary_component_count":world.boundary_components.len(),
            "boundary_edge_count":world.boundary_edges.len(),
            "discrete_digest":world.discrete_digest(),
            "generation_attempt":world.generation_attempt,
            "owner_count":world.owners.len(),
            "plate_count":world.plates.len(),
            "qualifications":qualifications,
            "world_id":world.world_id
        })
    );
    Ok(())
}

fn run_terrain_input(command: TerrainInputCommand) -> Result<(), Box<dyn Error>> {
    match command {
        TerrainInputCommand::Build {
            seed,
            spatial_release_dir,
            tectonic_release_dir,
            store_root,
            release_id,
            candidate_id,
            profile,
            output_namespace,
        } => {
            let spec = WorldSpec::earthlike(&seed)?;
            let spatial = open_spatial(&spatial_release_dir)?;
            let tectonic = open_tectonics(&tectonic_release_dir)?;
            let spatial_hash = sha256_hex(&canonical_json(&spatial)?);
            let tectonic_hash = tectonic.manifest_sha256.clone();
            let bundle = TerrainMaterialInputBundle::build(
                &spec,
                &spatial,
                spatial_hash,
                &tectonic.manifest,
                tectonic_hash,
            )?;
            let plan = TerrainMaterialRunPlan::new(profile.into(), &output_namespace)?;
            let release = M1InputRelease::new(release_id, bundle, plan, &tectonic.manifest);
            let published = publish_terrain_input(&store_root, &candidate_id, &release)?;
            println!("{}", serde_json::to_string(&published)?);
        }
        TerrainInputCommand::Inspect { release_dir } => {
            println!(
                "{}",
                serde_json::to_string(&open_terrain_input(&release_dir)?)?
            );
        }
        TerrainInputCommand::Validate { release_dir } => {
            let release = open_terrain_input(&release_dir)?;
            println!(
                "{}",
                serde_json::json!({"m1_release_id":release.m1_release_id,"status":"VALID"})
            );
        }
    }
    Ok(())
}

fn read_world_spec(path: &PathBuf) -> Result<WorldSpec, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let spec: WorldSpec = serde_json::from_slice(&bytes)?;
    if canonical_json(&spec)? != bytes {
        return Err("world specification is not canonical JSON".into());
    }
    spec.validate()?;
    Ok(spec)
}

impl From<PlateCountArg> for PlateCountPreference {
    fn from(value: PlateCountArg) -> Self {
        match value {
            PlateCountArg::Fewer => Self::Fewer,
            PlateCountArg::Standard => Self::Standard,
            PlateCountArg::More => Self::More,
        }
    }
}

impl From<ActivityArg> for TectonicActivityPreference {
    fn from(value: ActivityArg) -> Self {
        match value {
            ActivityArg::Lower => Self::Lower,
            ActivityArg::Standard => Self::Standard,
            ActivityArg::Higher => Self::Higher,
        }
    }
}

impl From<ProvenanceArg> for TectonicProvenance {
    fn from(value: ProvenanceArg) -> Self {
        match value {
            ProvenanceArg::FixedTest => Self::FixedTest,
            ProvenanceArg::RandomPrototype => Self::RandomPrototype,
        }
    }
}

impl From<RunProfileArg> for RunProfile {
    fn from(value: RunProfileArg) -> Self {
        match value {
            RunProfileArg::FixedTest => Self::FixedTest,
            RunProfileArg::Diagnostic => Self::Diagnostic,
            RunProfileArg::StandardGlobal => Self::StandardGlobal,
            RunProfileArg::RegionalRefinement => Self::RegionalRefinement,
        }
    }
}
