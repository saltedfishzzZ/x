//! E2.1 v2稳定板块骨架。

use super::{
    BoundaryComponent, BoundaryEdge, BoundaryId, FixedPlateInput, MAX_GENERATION_ATTEMPTS,
    PLATE_SITE_LEVEL, PlateId, TECTONIC_LEVEL, TectonicError, boundary_components, cell_index,
    cells_at_level, edge_corners, plate_areas, validate_domains,
};
use deterministic_numeric::{
    div_round_half_even as numeric_div_round_half_even, map_u53_to_range, quantize_i64,
};
use serde::{Deserialize, Serialize};
use sphere_core::{CardinalDirection, CellId, EdgeId, UnitVec3, Vec3};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;
use world_contracts::{canonical_json, sha256_hex};
use world_spec::v2::{GenerationIdentityV2, PLATE_SITE_MODEL_V2, WorldIntentSpecV2};
use world_spec::{NamedRandom, TectonicActivityPreference};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TectonicSkeletonId(pub String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlateRoleId(pub String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlateIdV2(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlateRoleV2 {
    pub plate_role_id: PlateRoleId,
    pub generation_rank: u16,
    pub site_cell_id: CellId,
    pub euler_axis_cell_id: CellId,
    pub speed_quantile_u53: u64,
    pub fixed_base_rate_microdegrees_per_million_years: Option<u32>,
    pub area_trillionths: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TectonicSkeletonV2 {
    pub tectonic_skeleton_id: TectonicSkeletonId,
    pub level: u8,
    pub generation_attempt: u8,
    pub plate_roles: Vec<PlateRoleV2>,
    pub owners: Vec<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityScaleV2 {
    pub numerator: u32,
    pub denominator: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlateMotionV2 {
    pub plate_role_id: PlateRoleId,
    pub plate_id: PlateIdV2,
    pub raw_base_rate_microdegrees_per_million_years: u32,
    pub raw_rate_vector_nanodegrees_per_million_years: [i64; 3],
    pub corrected_base_rate_vector_nanodegrees_per_million_years: [i64; 3],
    pub final_rate_vector_nanodegrees_per_million_years: [i64; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BoundaryKinematicTypeV2 {
    Convergent,
    Divergent,
    TransformDominant,
    KinematicallyIndeterminate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundaryKinematicSampleV2 {
    pub edge_id: EdgeId,
    pub boundary_id: BoundaryId,
    pub plate_a: PlateIdV2,
    pub plate_b: PlateIdV2,
    pub base_normal_rate_micrometres_per_year: i64,
    pub base_tangent_rate_micrometres_per_year: i64,
    pub normal_rate_micrometres_per_year: i64,
    pub tangent_rate_micrometres_per_year: i64,
    pub kinematic_type: BoundaryKinematicTypeV2,
    pub classification_confidence_millionths: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TectonicKinematicsV2 {
    pub generation_identity: String,
    pub tectonic_skeleton_id: TectonicSkeletonId,
    pub activity_scale: ActivityScaleV2,
    pub plate_motions: Vec<PlateMotionV2>,
    pub boundary_samples: Vec<BoundaryKinematicSampleV2>,
}

pub const TECTONIC_SHARD_COUNT_V2: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ShardOrderV2 {
    Forward,
    Reverse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfigV2 {
    pub worker_count: usize,
    pub shard_order: ShardOrderV2,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionEvidenceV2 {
    pub requested_worker_count: usize,
    pub participating_worker_count: usize,
    pub task_count: usize,
    pub shard_count: usize,
}

/// Derives the release-local plate identity from a stable natural role.
///
/// # Errors
/// Propagates canonical identity serialization failures.
pub fn derive_plate_id(
    generation: &GenerationIdentityV2,
    role: &PlateRoleId,
) -> Result<PlateIdV2, TectonicError> {
    let bytes = canonical_json(&("plate-v2", &generation.generation_identity, role))?;
    Ok(PlateIdV2(format!("plate-v2-{}", &sha256_hex(&bytes)[..24])))
}

/// Builds reference-frame-corrected plate motion and boundary samples.
///
/// # Errors
/// Rejects mismatched world or skeleton identities and malformed topology.
pub fn build_kinematics(
    intent: &WorldIntentSpecV2,
    generation: &GenerationIdentityV2,
    skeleton: &TectonicSkeletonV2,
) -> Result<TectonicKinematicsV2, TectonicError> {
    let intent_identity = intent.identity()?;
    if generation.world_id != intent_identity.world_id
        || skeleton.tectonic_skeleton_id != skeleton_id(intent)?
        || skeleton.level != TECTONIC_LEVEL
        || skeleton.plate_roles.is_empty()
    {
        return Err(TectonicError::InvalidTopology(
            "world, generation, or skeleton identity mismatch".into(),
        ));
    }
    let activity_scale = activity_scale(intent.tectonic_activity_preference);
    let raw_vectors = skeleton
        .plate_roles
        .iter()
        .map(|role| {
            let rate = role
                .fixed_base_rate_microdegrees_per_million_years
                .unwrap_or_else(|| base_rate_from_quantile(role.speed_quantile_u53));
            Ok((rate, rate_vector(role.euler_axis_cell_id, rate)?))
        })
        .collect::<Result<Vec<_>, TectonicError>>()?;
    let total_area = skeleton
        .plate_roles
        .iter()
        .map(|role| i128::from(role.area_trillionths))
        .sum::<i128>();
    if total_area <= 0 {
        return Err(TectonicError::InvalidTopology(
            "plate area weights are empty".into(),
        ));
    }
    let mut mean = [0_i64; 3];
    for component in 0..3 {
        let weighted = skeleton
            .plate_roles
            .iter()
            .zip(&raw_vectors)
            .map(|(role, (_, vector))| {
                i128::from(role.area_trillionths) * i128::from(vector[component])
            })
            .sum::<i128>();
        mean[component] = i64::try_from(
            numeric_div_round_half_even(weighted, total_area).map_err(numeric_error)?,
        )
        .map_err(|_| TectonicError::InvalidTopology("reference-frame mean exceeds i64".into()))?;
    }
    let plate_motions = skeleton
        .plate_roles
        .iter()
        .zip(raw_vectors)
        .map(|(role, (rate, raw))| {
            let corrected = [raw[0] - mean[0], raw[1] - mean[1], raw[2] - mean[2]];
            let final_vector = corrected.map(|component| {
                scale_integer(
                    component,
                    activity_scale.numerator,
                    activity_scale.denominator,
                )
            });
            Ok(PlateMotionV2 {
                plate_role_id: role.plate_role_id.clone(),
                plate_id: derive_plate_id(generation, &role.plate_role_id)?,
                raw_base_rate_microdegrees_per_million_years: rate,
                raw_rate_vector_nanodegrees_per_million_years: raw,
                corrected_base_rate_vector_nanodegrees_per_million_years: corrected,
                final_rate_vector_nanodegrees_per_million_years: final_vector,
            })
        })
        .collect::<Result<Vec<_>, TectonicError>>()?;
    let boundary_samples = build_boundary_samples(
        skeleton,
        &plate_motions,
        &generation.generation_identity,
        intent.radius_metres,
    )?;
    Ok(TectonicKinematicsV2 {
        generation_identity: generation.generation_identity.clone(),
        tectonic_skeleton_id: skeleton.tectonic_skeleton_id.clone(),
        activity_scale,
        plate_motions,
        boundary_samples,
    })
}

/// Builds the v2 natural-world plate skeleton without using activity preference.
///
/// # Errors
/// Rejects fixed-test intents and deterministic domain candidates that violate constraints.
pub fn build_natural_skeleton(
    intent: &WorldIntentSpecV2,
) -> Result<TectonicSkeletonV2, TectonicError> {
    build_natural_skeleton_with_execution(
        intent,
        ExecutionConfigV2 {
            worker_count: 1,
            shard_order: ShardOrderV2::Forward,
        },
    )
    .map(|(skeleton, _)| skeleton)
}

/// Builds the same natural skeleton with an explicit, evidenced worker pool.
///
/// # Errors
/// Rejects invalid worker counts and all ordinary skeleton generation failures.
pub fn build_natural_skeleton_with_execution(
    intent: &WorldIntentSpecV2,
    execution: ExecutionConfigV2,
) -> Result<(TectonicSkeletonV2, ExecutionEvidenceV2), TectonicError> {
    intent.validate()?;
    if intent.fixed_test.is_some() {
        return Err(TectonicError::InvalidTopology(
            "fixed-test intents require an explicit v2 fixed fixture".into(),
        ));
    }
    let identity = intent.identity()?;
    let random = NamedRandom::from_seed_key(&identity.seed_key)?;
    let skeleton_id = skeleton_id(intent)?;
    let plate_count = usize::from(intent.plate_count()?);
    let candidates = cells_at_level(PLATE_SITE_LEVEL)?;
    let cells = cells_at_level(TECTONIC_LEVEL)?;
    let mut last_failure = String::from("no candidate evaluated");

    for attempt in 0..MAX_GENERATION_ATTEMPTS {
        let sites = select_sites_v2(&random, &candidates, plate_count, attempt)?;
        if !site_separation_valid_v2(&sites) {
            "plate sites violate the 20 degree separation".clone_into(&mut last_failure);
            continue;
        }
        let role_ids = sites
            .iter()
            .enumerate()
            .map(|(rank, site)| plate_role_id(&skeleton_id, rank, *site))
            .collect::<Result<Vec<_>, _>>()?;
        let tie_break_ids = role_ids
            .iter()
            .map(|role| PlateId(role.0.clone()))
            .collect::<Vec<_>>();
        let (owners, evidence) =
            assign_owners_v2_parallel(&cells, &sites, &tie_break_ids, execution)?;
        let areas = plate_areas(&cells, &owners, plate_count);
        if let Err(reason) = validate_domains(TECTONIC_LEVEL, &owners, &areas, plate_count) {
            last_failure = reason;
            continue;
        }
        let mut roles = sites
            .iter()
            .zip(&role_ids)
            .enumerate()
            .zip(&areas)
            .map(|((rank, (site, role_id)), area)| {
                let axis_sample = random.bounded(
                    "tectonics-skeleton-v2",
                    &role_id.0,
                    "euler-axis-cell",
                    0,
                    u64::try_from(candidates.len()).unwrap_or(u64::MAX),
                )?;
                let axis_index = usize::try_from(axis_sample.value).map_err(|_| {
                    TectonicError::InvalidTopology("Euler axis sample is out of range".into())
                })?;
                let euler_axis_cell_id = *candidates.get(axis_index).ok_or_else(|| {
                    TectonicError::InvalidTopology("Euler axis sample is out of range".into())
                })?;
                Ok(PlateRoleV2 {
                    plate_role_id: role_id.clone(),
                    generation_rank: u16::try_from(rank).unwrap_or(u16::MAX),
                    site_cell_id: *site,
                    euler_axis_cell_id,
                    speed_quantile_u53: random.sample_u64(
                        "tectonics-skeleton-v2",
                        &role_id.0,
                        "speed-quantile",
                        0,
                    ) >> 11,
                    fixed_base_rate_microdegrees_per_million_years: None,
                    area_trillionths: area_trillionths(*area)?,
                })
            })
            .collect::<Result<Vec<_>, TectonicError>>()?;
        roles.sort_by(|left, right| left.plate_role_id.cmp(&right.plate_role_id));
        let positions = roles
            .iter()
            .enumerate()
            .map(|(index, role)| {
                (
                    role.plate_role_id.clone(),
                    u16::try_from(index).unwrap_or(u16::MAX),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let remapped = owners
            .iter()
            .map(|owner| positions[&role_ids[usize::from(*owner)]])
            .collect();
        return Ok((
            TectonicSkeletonV2 {
                tectonic_skeleton_id: skeleton_id,
                level: TECTONIC_LEVEL,
                generation_attempt: attempt + 1,
                plate_roles: roles,
                owners: remapped,
            },
            evidence,
        ));
    }
    Err(TectonicError::GenerationFailed(last_failure))
}

/// Builds a fixed-test skeleton whose explicit input bytes match the world intent.
///
/// # Errors
/// Rejects natural intents, fixture hash mismatches, duplicate sites, or invalid domains.
pub fn build_fixed_skeleton(
    intent: &WorldIntentSpecV2,
    fixed: &[FixedPlateInput],
) -> Result<TectonicSkeletonV2, TectonicError> {
    intent.validate()?;
    let descriptor = intent.fixed_test.as_ref().ok_or_else(|| {
        TectonicError::InvalidTopology("natural intents cannot use fixed plate input".into())
    })?;
    if sha256_hex(&canonical_json(&fixed)?) != descriptor.fixture_sha256 {
        return Err(TectonicError::InvalidTopology(
            "fixed plate input does not match its registered fixture hash".into(),
        ));
    }
    if fixed.len() != 6
        || fixed
            .iter()
            .any(|plate| plate.euler_rate_microdegrees_per_million_years == 0)
        || fixed
            .iter()
            .map(|plate| plate.site_cell_id)
            .collect::<BTreeSet<_>>()
            .len()
            != fixed.len()
        || fixed
            .iter()
            .map(|plate| &plate.plate_key)
            .collect::<BTreeSet<_>>()
            .len()
            != fixed.len()
    {
        return Err(TectonicError::InvalidTopology(
            "fixed test requires six unique sites, keys, and positive rates".into(),
        ));
    }

    let identity = intent.identity()?;
    let random = NamedRandom::from_seed_key(&identity.seed_key)?;
    let skeleton_id = skeleton_id(intent)?;
    let cells = cells_at_level(TECTONIC_LEVEL)?;
    let sites = fixed
        .iter()
        .map(|plate| plate.site_cell_id)
        .collect::<Vec<_>>();
    let role_ids = sites
        .iter()
        .enumerate()
        .map(|(rank, site)| plate_role_id(&skeleton_id, rank, *site))
        .collect::<Result<Vec<_>, _>>()?;
    let tie_break_ids = role_ids
        .iter()
        .map(|role| PlateId(role.0.clone()))
        .collect::<Vec<_>>();
    let owners = assign_owners_v2(&cells, &sites, &tie_break_ids);
    let areas = plate_areas(&cells, &owners, fixed.len());
    validate_domains(TECTONIC_LEVEL, &owners, &areas, fixed.len())
        .map_err(TectonicError::GenerationFailed)?;

    let mut roles = fixed
        .iter()
        .zip(&role_ids)
        .enumerate()
        .zip(&areas)
        .map(|((rank, (plate, role_id)), area)| {
            Ok(PlateRoleV2 {
                plate_role_id: role_id.clone(),
                generation_rank: u16::try_from(rank).unwrap_or(u16::MAX),
                site_cell_id: plate.site_cell_id,
                euler_axis_cell_id: plate.euler_axis_cell_id,
                speed_quantile_u53: random.sample_u64(
                    "tectonics-skeleton-v2",
                    &role_id.0,
                    "speed-quantile",
                    0,
                ) >> 11,
                fixed_base_rate_microdegrees_per_million_years: Some(
                    plate.euler_rate_microdegrees_per_million_years,
                ),
                area_trillionths: area_trillionths(*area)?,
            })
        })
        .collect::<Result<Vec<_>, TectonicError>>()?;
    roles.sort_by(|left, right| left.plate_role_id.cmp(&right.plate_role_id));
    let positions = roles
        .iter()
        .enumerate()
        .map(|(index, role)| {
            (
                role.plate_role_id.clone(),
                u16::try_from(index).unwrap_or(u16::MAX),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let remapped = owners
        .iter()
        .map(|owner| positions[&role_ids[usize::from(*owner)]])
        .collect();
    Ok(TectonicSkeletonV2 {
        tectonic_skeleton_id: skeleton_id,
        level: TECTONIC_LEVEL,
        generation_attempt: 1,
        plate_roles: roles,
        owners: remapped,
    })
}

fn skeleton_id(intent: &WorldIntentSpecV2) -> Result<TectonicSkeletonId, TectonicError> {
    let identity = intent.identity()?;
    let bytes = canonical_json(&(
        "tectonic-skeleton-v2",
        identity.seed_key,
        intent.plate_count_preference,
        PLATE_SITE_MODEL_V2,
        &intent.fixed_test,
    ))?;
    Ok(TectonicSkeletonId(format!(
        "skeleton-v2-{}",
        &sha256_hex(&bytes)[..24]
    )))
}

fn plate_role_id(
    skeleton_id: &TectonicSkeletonId,
    generation_rank: usize,
    site: CellId,
) -> Result<PlateRoleId, TectonicError> {
    let bytes = canonical_json(&(
        "plate-role-v2",
        skeleton_id,
        u64::try_from(generation_rank).unwrap_or(u64::MAX),
        site,
    ))?;
    Ok(PlateRoleId(format!(
        "plate-role-v2-{}",
        &sha256_hex(&bytes)[..24]
    )))
}

fn select_sites_v2(
    random: &NamedRandom,
    candidates: &[CellId],
    count: usize,
    attempt: u8,
) -> Result<Vec<CellId>, TectonicError> {
    let sample = random.bounded(
        "tectonics-skeleton-v2",
        "world",
        "first-plate-site",
        u64::from(attempt),
        u64::try_from(candidates.len()).unwrap_or(u64::MAX),
    )?;
    let first_index = usize::try_from(sample.value)
        .map_err(|_| TectonicError::InvalidTopology("plate site sample is out of range".into()))?;
    let first = *candidates.get(first_index).ok_or_else(|| {
        TectonicError::InvalidTopology("plate site sample is out of range".into())
    })?;
    let mut sites = vec![first];
    let mut selected = BTreeSet::from([first]);
    while sites.len() < count {
        let mut best: Option<(i64, CellId)> = None;
        for candidate in candidates.iter().copied() {
            if selected.contains(&candidate) {
                continue;
            }
            let nearest_dot = sites
                .iter()
                .map(|site| quantized_dot_v2(candidate.center_unit(), site.center_unit()))
                .max()
                .unwrap_or(i64::MAX);
            if best.is_none_or(|current| (nearest_dot, candidate) < current) {
                best = Some((nearest_dot, candidate));
            }
        }
        let next = best
            .map(|(_, cell)| cell)
            .ok_or_else(|| TectonicError::GenerationFailed("site candidates exhausted".into()))?;
        selected.insert(next);
        sites.push(next);
    }
    Ok(sites)
}

fn assign_owners_v2(cells: &[CellId], sites: &[CellId], role_ids: &[PlateId]) -> Vec<u16> {
    let site_vectors = sites
        .iter()
        .map(|site| site.center_unit())
        .collect::<Vec<_>>();
    cells
        .iter()
        .map(|cell| {
            let center = cell.center_unit();
            let mut best = (i64::MIN, &role_ids[0], 0_usize);
            for (index, site) in site_vectors.iter().enumerate() {
                let candidate = (quantized_dot_v2(center, *site), &role_ids[index], index);
                if candidate.0 > best.0 || (candidate.0 == best.0 && candidate.1 < best.1) {
                    best = candidate;
                }
            }
            u16::try_from(best.2).unwrap_or(u16::MAX)
        })
        .collect()
}

fn assign_owners_v2_parallel(
    cells: &[CellId],
    sites: &[CellId],
    role_ids: &[PlateId],
    execution: ExecutionConfigV2,
) -> Result<(Vec<u16>, ExecutionEvidenceV2), TectonicError> {
    let shard_count = TECTONIC_SHARD_COUNT_V2.min(cells.len());
    if execution.worker_count == 0 || execution.worker_count > shard_count {
        return Err(TectonicError::InvalidTopology(format!(
            "worker count must be within 1..={shard_count}"
        )));
    }
    let worker_outputs = std::thread::scope(|scope| {
        let handles = (0..execution.worker_count)
            .map(|worker| {
                scope.spawn(move || {
                    let mut assigned_shards = (worker..shard_count)
                        .step_by(execution.worker_count)
                        .collect::<Vec<_>>();
                    if execution.shard_order == ShardOrderV2::Reverse {
                        assigned_shards.reverse();
                    }
                    assigned_shards
                        .into_iter()
                        .map(|shard| {
                            let start = cells.len() * shard / shard_count;
                            let end = cells.len() * (shard + 1) / shard_count;
                            (shard, assign_owners_v2(&cells[start..end], sites, role_ids))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().map_err(|_| {
                    TectonicError::GenerationFailed("tectonic worker panicked".to_owned())
                })
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let participating_worker_count = worker_outputs
        .iter()
        .filter(|shards| !shards.is_empty())
        .count();
    let mut shards = worker_outputs.into_iter().flatten().collect::<Vec<_>>();
    shards.sort_by_key(|(shard, _)| *shard);
    let owners = shards
        .into_iter()
        .flat_map(|(_, owners)| owners)
        .collect::<Vec<_>>();
    if owners.len() != cells.len() || participating_worker_count != execution.worker_count {
        return Err(TectonicError::GenerationFailed(
            "parallel execution did not cover every task and worker".to_owned(),
        ));
    }
    Ok((
        owners,
        ExecutionEvidenceV2 {
            requested_worker_count: execution.worker_count,
            participating_worker_count,
            task_count: cells.len(),
            shard_count,
        },
    ))
}

fn area_trillionths(area: f64) -> Result<u64, TectonicError> {
    let value = quantize_i64(area / (4.0 * std::f64::consts::PI), 1.0e12).map_err(numeric_error)?;
    u64::try_from(value)
        .map_err(|_| TectonicError::InvalidTopology("plate area became negative".to_owned()))
}

const fn activity_scale(preference: TectonicActivityPreference) -> ActivityScaleV2 {
    match preference {
        TectonicActivityPreference::Lower => ActivityScaleV2 {
            numerator: 1,
            denominator: 2,
        },
        TectonicActivityPreference::Standard => ActivityScaleV2 {
            numerator: 1,
            denominator: 1,
        },
        TectonicActivityPreference::Higher => ActivityScaleV2 {
            numerator: 3,
            denominator: 2,
        },
    }
}

fn base_rate_from_quantile(quantile_u53: u64) -> u32 {
    const MINIMUM: u32 = 200_000;
    u32::try_from(
        map_u53_to_range(quantile_u53, u64::from(MINIMUM), 700_001)
            .expect("the constant tectonic speed range is valid"),
    )
    .expect("the constant tectonic speed range fits u32")
}

fn rate_vector(
    axis: CellId,
    rate_microdegrees_per_million_years: u32,
) -> Result<[i64; 3], TectonicError> {
    let vector = axis.center_unit().as_vec3();
    let scale = f64::from(rate_microdegrees_per_million_years) * 1_000.0;
    Ok([
        quantize_i64(vector.x, scale).map_err(numeric_error)?,
        quantize_i64(vector.y, scale).map_err(numeric_error)?,
        quantize_i64(vector.z, scale).map_err(numeric_error)?,
    ])
}

fn scale_integer(value: i64, numerator: u32, denominator: u32) -> i64 {
    let scaled = numeric_div_round_half_even(
        i128::from(value) * i128::from(numerator),
        i128::from(denominator),
    )
    .expect("positive u32 activity scale cannot fail");
    i64::try_from(scaled).unwrap_or_else(|_| {
        if scaled.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

fn build_boundary_samples(
    skeleton: &TectonicSkeletonV2,
    motions: &[PlateMotionV2],
    generation_identity: &str,
    radius_metres: u64,
) -> Result<Vec<BoundaryKinematicSampleV2>, TectonicError> {
    if motions.len() != skeleton.plate_roles.len() {
        return Err(TectonicError::InvalidTopology(
            "plate motion count does not match the skeleton".into(),
        ));
    }
    let cells = cells_at_level(skeleton.level)?;
    if cells.len() != skeleton.owners.len() {
        return Err(TectonicError::InvalidTopology(
            "owner count does not cover the tectonic sphere".into(),
        ));
    }
    let mut edges = Vec::new();
    for (index, cell) in cells.iter().copied().enumerate() {
        for direction in CardinalDirection::ALL {
            let neighbour = cell.neighbour(direction)?;
            if cell >= neighbour {
                continue;
            }
            let neighbour_index = cell_index(neighbour);
            let left = usize::from(skeleton.owners[index]);
            let right = usize::from(skeleton.owners[neighbour_index]);
            if left == right {
                continue;
            }
            let left_motion = motions.get(left).ok_or_else(|| {
                TectonicError::InvalidTopology("left plate motion is missing".into())
            })?;
            let right_motion = motions.get(right).ok_or_else(|| {
                TectonicError::InvalidTopology("right plate motion is missing".into())
            })?;
            let (plate_a, plate_b, cell_a, cell_b) =
                if left_motion.plate_id <= right_motion.plate_id {
                    (left_motion, right_motion, cell, neighbour)
                } else {
                    (right_motion, left_motion, neighbour, cell)
                };
            let relation = cell.neighbour_relation(direction)?;
            let corners = edge_corners(direction);
            let mut vertices = [cell.vertex_id(corners[0])?, cell.vertex_id(corners[1])?];
            vertices.sort();
            edges.push(BoundaryEdge {
                edge_id: relation.shared_edge_id,
                plate_a: PlateId(plate_a.plate_id.0.clone()),
                plate_b: PlateId(plate_b.plate_id.0.clone()),
                cell_a,
                cell_b,
                vertices,
                component_index: u32::MAX,
            });
        }
    }
    edges.sort_by_key(|edge| edge.edge_id);
    let (components, vertex_degrees) = boundary_components(generation_identity, &mut edges)?;
    if let Some((_, degree)) = vertex_degrees
        .iter()
        .find(|(_, degree)| **degree == 1 || **degree >= 4)
    {
        return Err(TectonicError::InvalidTopology(format!(
            "boundary vertex has unsupported degree {degree}"
        )));
    }
    let motion_by_id = motions
        .iter()
        .map(|motion| (motion.plate_id.0.as_str(), motion))
        .collect::<BTreeMap<_, _>>();
    edges
        .iter()
        .map(|edge| {
            let component = components
                .get(usize::try_from(edge.component_index).unwrap_or(usize::MAX))
                .ok_or_else(|| {
                    TectonicError::InvalidTopology("edge component is missing".into())
                })?;
            let plate_a = motion_by_id.get(edge.plate_a.0.as_str()).ok_or_else(|| {
                TectonicError::InvalidTopology("plate A motion is missing".into())
            })?;
            let plate_b = motion_by_id.get(edge.plate_b.0.as_str()).ok_or_else(|| {
                TectonicError::InvalidTopology("plate B motion is missing".into())
            })?;
            boundary_sample(edge, component, plate_a, plate_b, radius_metres)
        })
        .collect()
}

fn boundary_sample(
    edge: &BoundaryEdge,
    component: &BoundaryComponent,
    plate_a: &PlateMotionV2,
    plate_b: &PlateMotionV2,
    radius_metres: u64,
) -> Result<BoundaryKinematicSampleV2, TectonicError> {
    let endpoint_cells = edge.edge_id.cells();
    let shared_direction = CardinalDirection::ALL
        .into_iter()
        .find(|direction| {
            endpoint_cells[0]
                .neighbour(*direction)
                .is_ok_and(|value| value == endpoint_cells[1])
        })
        .ok_or_else(|| TectonicError::InvalidTopology("edge cells are not adjacent".into()))?;
    let endpoints = endpoint_cells[0].edge_endpoints(shared_direction);
    let position = UnitVec3::new(endpoints[0].as_vec3() + endpoints[1].as_vec3())?;
    let delta = edge.cell_b.center() - edge.cell_a.center();
    let radial = position.as_vec3() * delta.dot(position.as_vec3());
    let normal_axis = UnitVec3::new(delta - radial)?;
    let tangent_axis = UnitVec3::new(normal_axis.cross(position))?;
    let base_relative = velocity_from_rate_vector(
        plate_b.corrected_base_rate_vector_nanodegrees_per_million_years,
        position,
        radius_metres,
    ) - velocity_from_rate_vector(
        plate_a.corrected_base_rate_vector_nanodegrees_per_million_years,
        position,
        radius_metres,
    );
    let final_relative = velocity_from_rate_vector(
        plate_b.final_rate_vector_nanodegrees_per_million_years,
        position,
        radius_metres,
    ) - velocity_from_rate_vector(
        plate_a.final_rate_vector_nanodegrees_per_million_years,
        position,
        radius_metres,
    );
    let base_normal = rounded_i64(base_relative.dot(normal_axis.as_vec3()))?;
    let base_tangent = rounded_i64(base_relative.dot(tangent_axis.as_vec3()))?;
    let normal = rounded_i64(final_relative.dot(normal_axis.as_vec3()))?;
    let tangent = rounded_i64(final_relative.dot(tangent_axis.as_vec3()))?;
    let (kinematic_type, confidence) = classify_direction(base_normal, base_tangent);
    Ok(BoundaryKinematicSampleV2 {
        edge_id: edge.edge_id,
        boundary_id: component.boundary_id.clone(),
        plate_a: plate_a.plate_id.clone(),
        plate_b: plate_b.plate_id.clone(),
        base_normal_rate_micrometres_per_year: base_normal,
        base_tangent_rate_micrometres_per_year: base_tangent,
        normal_rate_micrometres_per_year: normal,
        tangent_rate_micrometres_per_year: tangent,
        kinematic_type,
        classification_confidence_millionths: confidence,
    })
}

fn velocity_from_rate_vector(
    rate_vector_nanodegrees_per_million_years: [i64; 3],
    position: UnitVec3,
    radius_metres: u64,
) -> Vec3 {
    #[allow(clippy::cast_precision_loss)]
    let angular = Vec3::new(
        rate_vector_nanodegrees_per_million_years[0] as f64,
        rate_vector_nanodegrees_per_million_years[1] as f64,
        rate_vector_nanodegrees_per_million_years[2] as f64,
    );
    #[allow(clippy::cast_precision_loss)]
    let scale = radius_metres as f64 * PI / 180.0 * 1.0e-9;
    angular.cross(position.as_vec3()) * scale
}

fn rounded_i64(value: f64) -> Result<i64, TectonicError> {
    quantize_i64(value, 1.0).map_err(numeric_error)
}

fn quantized_dot_v2(left: UnitVec3, right: UnitVec3) -> i64 {
    quantize_i64(left.dot(right), 1.0e15)
        .expect("unit-vector dot products are finite and within the quantizer range")
}

fn site_separation_valid_v2(sites: &[CellId]) -> bool {
    // round-half-to-even(cos(20°) * 1e15), frozen by the v2 numeric protocol.
    const MAXIMUM_DOT_Q15: i64 = 939_692_620_785_908;
    sites.iter().enumerate().all(|(index, left)| {
        sites[index + 1..].iter().all(|right| {
            quantized_dot_v2(left.center_unit(), right.center_unit()) <= MAXIMUM_DOT_Q15
        })
    })
}

#[allow(clippy::needless_pass_by_value)]
fn numeric_error(error: deterministic_numeric::NumericError) -> TectonicError {
    TectonicError::InvalidTopology(format!("deterministic numeric protocol failed: {error}"))
}

fn classify_direction(normal: i64, tangent: i64) -> (BoundaryKinematicTypeV2, u32) {
    let normal_magnitude = normal.unsigned_abs();
    let tangent_magnitude = tangent.unsigned_abs();
    if normal_magnitude <= 1 && tangent_magnitude <= 1 {
        return (BoundaryKinematicTypeV2::KinematicallyIndeterminate, 0);
    }
    let total = u128::from(normal_magnitude) + u128::from(tangent_magnitude);
    let dominant = normal_magnitude.max(tangent_magnitude);
    let confidence = u32::try_from((u128::from(dominant) * 1_000_000) / total).unwrap_or(1_000_000);
    if u128::from(normal_magnitude) * 2 >= u128::from(tangent_magnitude) {
        match normal.cmp(&0) {
            std::cmp::Ordering::Less => (BoundaryKinematicTypeV2::Convergent, confidence),
            std::cmp::Ordering::Greater => (BoundaryKinematicTypeV2::Divergent, confidence),
            std::cmp::Ordering::Equal => (BoundaryKinematicTypeV2::TransformDominant, confidence),
        }
    } else {
        (BoundaryKinematicTypeV2::TransformDominant, confidence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pv01_fixed_input;
    use serde::Deserialize;
    use world_spec::TectonicActivityPreference;
    use world_spec::v2::{GenerationSpecV2, ReleaseRefV2};

    #[derive(Deserialize)]
    struct ActivityFixture {
        fixture_id: String,
        schema: String,
        seed: String,
        worker_counts: Vec<usize>,
        shard_orders: Vec<ShardOrderV2>,
        expected_shard_count: usize,
        activity_scales: BTreeMap<String, [u32; 2]>,
    }

    #[test]
    fn activity_fixture_is_executable() {
        let fixture: ActivityFixture = serde_json::from_slice(include_bytes!(
            "../../../fixtures/e2.1/tectonics/E21T01-activity-metamorphic.json"
        ))
        .unwrap();
        assert_eq!(fixture.fixture_id, "E21-T01");
        assert_eq!(fixture.schema, "e2.1-activity-metamorphic-fixture-v1");
        let intent = WorldIntentSpecV2::earthlike_natural(&fixture.seed).unwrap();
        let expected = build_natural_skeleton(&intent).unwrap();
        for worker_count in fixture.worker_counts {
            for shard_order in &fixture.shard_orders {
                let (actual, evidence) = build_natural_skeleton_with_execution(
                    &intent,
                    ExecutionConfigV2 {
                        worker_count,
                        shard_order: *shard_order,
                    },
                )
                .unwrap();
                assert_eq!(actual, expected);
                assert_eq!(evidence.shard_count, fixture.expected_shard_count);
                assert_eq!(evidence.participating_worker_count, worker_count);
            }
        }
        for (name, expected) in fixture.activity_scales {
            let preference = match name.as_str() {
                "LOWER" => TectonicActivityPreference::Lower,
                "STANDARD" => TectonicActivityPreference::Standard,
                "HIGHER" => TectonicActivityPreference::Higher,
                _ => panic!("unknown fixture activity: {name}"),
            };
            let actual = activity_scale(preference);
            assert_eq!([actual.numerator, actual.denominator], expected);
        }
    }

    #[test]
    fn fixed_shards_are_identical_across_workers_and_completion_orders() {
        let intent = WorldIntentSpecV2::earthlike_natural("v2-real-parallelism").unwrap();
        let mut expected = None;
        let mut expected_bytes = None;
        for worker_count in [1, 2, 4] {
            for shard_order in [ShardOrderV2::Forward, ShardOrderV2::Reverse] {
                let (skeleton, evidence) = build_natural_skeleton_with_execution(
                    &intent,
                    ExecutionConfigV2 {
                        worker_count,
                        shard_order,
                    },
                )
                .unwrap();
                assert_eq!(evidence.requested_worker_count, worker_count);
                assert_eq!(evidence.participating_worker_count, worker_count);
                assert_eq!(evidence.shard_count, TECTONIC_SHARD_COUNT_V2);
                assert_eq!(evidence.task_count, skeleton.owners.len());
                let bytes = canonical_json(&skeleton).unwrap();
                if let Some(reference) = &expected {
                    assert_eq!(&skeleton, reference);
                    assert_eq!(Some(&bytes), expected_bytes.as_ref());
                } else {
                    expected_bytes = Some(bytes);
                    expected = Some(skeleton);
                }
            }
        }
    }

    #[test]
    fn activity_preference_does_not_change_the_v2_skeleton() {
        let mut lower = WorldIntentSpecV2::earthlike_natural("v2-activity-skeleton").unwrap();
        lower.tectonic_activity_preference = TectonicActivityPreference::Lower;
        let mut standard = lower.clone();
        standard.tectonic_activity_preference = TectonicActivityPreference::Standard;
        let mut higher = lower.clone();
        higher.tectonic_activity_preference = TectonicActivityPreference::Higher;

        let lower_world = lower.identity().unwrap();
        let standard_world = standard.identity().unwrap();
        let higher_world = higher.identity().unwrap();
        assert_ne!(lower_world.world_id, standard_world.world_id);
        assert_ne!(standard_world.world_id, higher_world.world_id);

        let lower_skeleton = build_natural_skeleton(&lower).unwrap();
        let standard_skeleton = build_natural_skeleton(&standard).unwrap();
        let higher_skeleton = build_natural_skeleton(&higher).unwrap();
        assert_eq!(lower_skeleton, standard_skeleton);
        assert_eq!(standard_skeleton, higher_skeleton);

        let spatial = ReleaseRefV2 {
            release_id: "spatial-v2".to_owned(),
            manifest_sha256: "a".repeat(64),
        };
        let models = BTreeMap::from([("tectonics".to_owned(), "tectonics-v2".to_owned())]);
        let lower_generation = GenerationSpecV2::new(&lower, spatial.clone(), models.clone())
            .unwrap()
            .identity(&lower)
            .unwrap();
        let higher_generation = GenerationSpecV2::new(&higher, spatial, models)
            .unwrap()
            .identity(&higher)
            .unwrap();
        let role = &lower_skeleton.plate_roles[0].plate_role_id;
        assert_eq!(role, &higher_skeleton.plate_roles[0].plate_role_id);
        assert_ne!(
            derive_plate_id(&lower_generation, role).unwrap(),
            derive_plate_id(&higher_generation, role).unwrap()
        );

        let lower_motion = build_kinematics(&lower, &lower_generation, &lower_skeleton).unwrap();
        let higher_motion =
            build_kinematics(&higher, &higher_generation, &higher_skeleton).unwrap();
        assert_eq!(
            lower_motion.activity_scale,
            ActivityScaleV2 {
                numerator: 1,
                denominator: 2
            }
        );
        assert_eq!(
            higher_motion.activity_scale,
            ActivityScaleV2 {
                numerator: 3,
                denominator: 2
            }
        );
        for (low, high) in lower_motion
            .plate_motions
            .iter()
            .zip(&higher_motion.plate_motions)
        {
            assert_eq!(low.plate_role_id, high.plate_role_id);
            assert_eq!(
                low.corrected_base_rate_vector_nanodegrees_per_million_years,
                high.corrected_base_rate_vector_nanodegrees_per_million_years
            );
            for component in 0..3 {
                let base = low.corrected_base_rate_vector_nanodegrees_per_million_years[component];
                assert_eq!(
                    low.final_rate_vector_nanodegrees_per_million_years[component],
                    scale_integer(base, 1, 2)
                );
                assert_eq!(
                    high.final_rate_vector_nanodegrees_per_million_years[component],
                    scale_integer(base, 3, 2)
                );
            }
        }
        assert_eq!(
            lower_motion.boundary_samples.len(),
            higher_motion.boundary_samples.len()
        );
        for (low, high) in lower_motion
            .boundary_samples
            .iter()
            .zip(&higher_motion.boundary_samples)
        {
            assert_eq!(low.edge_id, high.edge_id);
            assert_eq!(low.kinematic_type, high.kinematic_type);
            assert_eq!(
                low.base_normal_rate_micrometres_per_year,
                high.base_normal_rate_micrometres_per_year
            );
            assert_eq!(
                low.base_tangent_rate_micrometres_per_year,
                high.base_tangent_rate_micrometres_per_year
            );
        }
    }

    #[test]
    fn fixed_fixture_hash_enters_and_protects_the_skeleton() {
        let fixed = pv01_fixed_input().unwrap();
        let fixed_hash = sha256_hex(&canonical_json(&fixed).unwrap());
        let intent =
            WorldIntentSpecV2::earthlike_fixed_test("PV01-E2.1-v2", "PV01-E2.1-v2", &fixed_hash)
                .unwrap();
        let skeleton = build_fixed_skeleton(&intent, &fixed).unwrap();

        let mut changed = fixed.clone();
        changed[0].euler_rate_microdegrees_per_million_years += 1;
        assert!(build_fixed_skeleton(&intent, &changed).is_err());
        let changed_hash = sha256_hex(&canonical_json(&changed).unwrap());
        let changed_intent =
            WorldIntentSpecV2::earthlike_fixed_test("PV01-E2.1-v2", "PV01-E2.1-v2", changed_hash)
                .unwrap();
        let changed_skeleton = build_fixed_skeleton(&changed_intent, &changed).unwrap();
        assert_ne!(
            skeleton.tectonic_skeleton_id,
            changed_skeleton.tectonic_skeleton_id
        );
    }
}
