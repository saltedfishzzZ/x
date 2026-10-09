//! E2 构造核心：主板块球面域、共享边界和欧拉运动代理。

use serde::{Deserialize, Serialize};
use sphere_core::{
    CardinalDirection, CellId, CubeFace, EdgeId, SphereError, UnitVec3, Vec3, VertexId,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::f64::consts::PI;
use thiserror::Error;
use world_contracts::{canonical_json, sha256_hex};
use world_spec::{NamedRandom, WorldSpec, WorldSpecError};

pub mod v2;

pub const PLATE_SITE_LEVEL: u8 = 6;
pub const TECTONIC_LEVEL: u8 = 8;
pub const MAX_GENERATION_ATTEMPTS: u8 = 8;
pub const RATE_DEADBAND_MICROMETRES_PER_YEAR: i64 = 5_000;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlateId(pub String);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BoundaryId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Plate {
    pub plate_id: PlateId,
    pub site_cell_id: CellId,
    pub euler_axis_cell_id: CellId,
    pub euler_rate_microdegrees_per_million_years: u32,
    pub area_trillionths: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundaryEdge {
    pub edge_id: EdgeId,
    pub plate_a: PlateId,
    pub plate_b: PlateId,
    pub cell_a: CellId,
    pub cell_b: CellId,
    pub vertices: [VertexId; 2],
    pub component_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundaryComponent {
    pub boundary_id: BoundaryId,
    pub plate_a: PlateId,
    pub plate_b: PlateId,
    pub minimum_edge_id: EdgeId,
    pub edge_indices: Vec<u32>,
    pub is_loop: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BoundaryQualification {
    Convergent,
    Divergent,
    Stable,
    Unresolved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BoundaryKinematicSample {
    pub edge_id: EdgeId,
    pub boundary_id: BoundaryId,
    pub plate_a: PlateId,
    pub plate_b: PlateId,
    pub normal_rate_micrometres_per_year: i64,
    pub tangent_rate_micrometres_per_year: i64,
    pub tangent_significant: bool,
    pub qualification: BoundaryQualification,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConstructionMode {
    FixedTest,
    RandomPrototype,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FixedPlateInput {
    pub plate_key: String,
    pub site_cell_id: CellId,
    pub euler_axis_cell_id: CellId,
    pub euler_rate_microdegrees_per_million_years: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TectonicWorld {
    pub world_id: String,
    pub generation_identity: String,
    pub level: u8,
    pub generation_attempt: u8,
    pub construction_mode: ConstructionMode,
    pub plates: Vec<Plate>,
    pub owners: Vec<u16>,
    pub boundary_edges: Vec<BoundaryEdge>,
    pub boundary_components: Vec<BoundaryComponent>,
    pub kinematic_samples: Vec<BoundaryKinematicSample>,
}

#[derive(Debug, Error)]
pub enum TectonicError {
    #[error("world specification failed: {0}")]
    World(#[from] WorldSpecError),
    #[error("sphere operation failed: {0}")]
    Sphere(#[from] SphereError),
    #[error("canonical identity failed: {0}")]
    Contract(#[from] world_contracts::ContractError),
    #[error("cell {0} is outside the materialized tectonic field")]
    CellOutsideField(CellId),
    #[error("plate generation failed after eight attempts: {0}")]
    GenerationFailed(String),
    #[error("invalid tectonic topology: {0}")]
    InvalidTopology(String),
}

/// Builds the formal E2 L8 tectonic prototype.
///
/// # Errors
/// Returns a deterministic validation error when all eight candidates fail.
pub fn build(spec: &WorldSpec) -> Result<TectonicWorld, TectonicError> {
    build_at_level(spec, TECTONIC_LEVEL)
}

/// Recomputes all authoritative domain, boundary, and kinematic invariants.
///
/// # Errors
/// Rejects malformed owners, plate areas, topology, component identities, or motion samples.
pub fn validate(world: &TectonicWorld, radius_metres: u64) -> Result<(), TectonicError> {
    if world.level != TECTONIC_LEVEL
        || world.plates.is_empty()
        || world
            .owners
            .iter()
            .any(|owner| usize::from(*owner) >= world.plates.len())
        || !world
            .plates
            .windows(2)
            .all(|pair| pair[0].plate_id < pair[1].plate_id)
    {
        return Err(TectonicError::InvalidTopology(
            "level, plate order, or owner index is invalid".into(),
        ));
    }
    let cells = cells_at_level(world.level)?;
    if cells.len() != world.owners.len() {
        return Err(TectonicError::InvalidTopology(
            "owner count does not cover the L8 sphere".into(),
        ));
    }
    let areas = plate_areas(&cells, &world.owners, world.plates.len());
    validate_domains(world.level, &world.owners, &areas, world.plates.len())
        .map_err(TectonicError::InvalidTopology)?;
    for (plate, area) in world.plates.iter().zip(areas) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let expected = (area / (4.0 * PI) * 1.0e12).round() as u64;
        if plate.area_trillionths != expected {
            return Err(TectonicError::InvalidTopology(
                "stored plate area does not match owners".into(),
            ));
        }
    }
    let mut reconstructed = world.clone();
    reconstructed.boundary_edges.clear();
    reconstructed.boundary_components.clear();
    reconstructed.kinematic_samples.clear();
    populate_boundaries_and_motion(&mut reconstructed, radius_metres)?;
    if reconstructed.boundary_edges != world.boundary_edges
        || reconstructed.boundary_components != world.boundary_components
        || reconstructed.kinematic_samples != world.kinematic_samples
    {
        return Err(TectonicError::InvalidTopology(
            "boundary topology or motion does not match owners".into(),
        ));
    }
    Ok(())
}

fn build_at_level(spec: &WorldSpec, level: u8) -> Result<TectonicWorld, TectonicError> {
    let identity = spec.identity()?;
    let random = NamedRandom::from_spec(spec)?;
    let plate_count = usize::from(spec.plate_count()?);
    let candidates = cells_at_level(PLATE_SITE_LEVEL)?;
    let cells = cells_at_level(level)?;
    let mut last_failure = String::from("no candidate evaluated");

    for attempt in 0..MAX_GENERATION_ATTEMPTS {
        let sites = select_sites(&random, &candidates, plate_count, attempt)?;
        if !site_separation_valid(&sites) {
            "plate sites violate the 20 degree separation".clone_into(&mut last_failure);
            continue;
        }
        let plate_ids = sites
            .iter()
            .map(|site| plate_id(&identity.world_id, *site))
            .collect::<Result<Vec<_>, _>>()?;
        let owners = assign_owners(&cells, &sites, &plate_ids);
        let areas = plate_areas(&cells, &owners, plate_count);
        if let Err(reason) = validate_domains(level, &owners, &areas, plate_count) {
            last_failure = reason;
            continue;
        }
        let mut plates =
            build_plates(spec, &random, &candidates, sites, plate_ids.clone(), &areas)?;
        plates.sort_by(|left, right| left.plate_id.cmp(&right.plate_id));
        let positions = plates
            .iter()
            .enumerate()
            .map(|(index, plate)| {
                (
                    plate.plate_id.clone(),
                    u16::try_from(index).unwrap_or(u16::MAX),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let remapped = owners
            .iter()
            .map(|owner| positions[&plate_ids[usize::from(*owner)]])
            .collect();
        let mut world = TectonicWorld {
            world_id: identity.world_id.clone(),
            generation_identity: identity.generation_identity.clone(),
            level,
            generation_attempt: attempt + 1,
            construction_mode: ConstructionMode::RandomPrototype,
            plates,
            owners: remapped,
            boundary_edges: Vec::new(),
            boundary_components: Vec::new(),
            kinematic_samples: Vec::new(),
        };
        match populate_boundaries_and_motion(&mut world, spec.radius_metres) {
            Ok(()) => return Ok(world),
            Err(error) => last_failure = error.to_string(),
        }
    }
    Err(TectonicError::GenerationFailed(last_failure))
}

/// Returns the six explicitly fixed PV01 plate inputs.
///
/// # Errors
/// Propagates an impossible L6 identity construction error.
pub fn pv01_fixed_input() -> Result<Vec<FixedPlateInput>, TectonicError> {
    let sites = CubeFace::ALL
        .into_iter()
        .map(|face| CellId::new(face, PLATE_SITE_LEVEL, 31, 31))
        .collect::<Result<Vec<_>, _>>()?;
    let axes = [
        CubeFace::PosZ,
        CubeFace::NegZ,
        CubeFace::PosZ,
        CubeFace::NegX,
        CubeFace::PosY,
        CubeFace::NegY,
    ];
    let rates = [600_000, 600_000, 600_000, 500_000, 300_000, 300_000];
    sites
        .into_iter()
        .zip(axes)
        .zip(rates)
        .enumerate()
        .map(|(index, ((site, axis_face), rate))| {
            Ok(FixedPlateInput {
                plate_key: format!("P{index}"),
                site_cell_id: site,
                euler_axis_cell_id: CellId::new(axis_face, PLATE_SITE_LEVEL, 31, 31)?,
                euler_rate_microdegrees_per_million_years: rate,
            })
        })
        .collect()
}

/// Builds PV01 from explicit fixed sites, axes, and positive integer rates.
///
/// # Errors
/// Rejects duplicate sites, invalid rates, domain constraints, or boundary topology.
pub fn build_fixed(
    spec: &WorldSpec,
    fixed: &[FixedPlateInput],
) -> Result<TectonicWorld, TectonicError> {
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
            "fixed test requires six unique sites and positive rates".into(),
        ));
    }
    let identity = spec.identity()?;
    let cells = cells_at_level(TECTONIC_LEVEL)?;
    let sites = fixed
        .iter()
        .map(|plate| plate.site_cell_id)
        .collect::<Vec<_>>();
    let original_ids = fixed
        .iter()
        .map(|plate| fixed_plate_id(&identity.world_id, &plate.plate_key))
        .collect::<Result<Vec<_>, _>>()?;
    let owners = assign_owners(&cells, &sites, &original_ids);
    let areas = plate_areas(&cells, &owners, fixed.len());
    validate_domains(TECTONIC_LEVEL, &owners, &areas, fixed.len())
        .map_err(TectonicError::GenerationFailed)?;
    let mut plates = fixed
        .iter()
        .zip(&original_ids)
        .zip(&areas)
        .map(|((input, id), area)| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let area_trillionths = (*area / (4.0 * PI) * 1.0e12).round() as u64;
            Plate {
                plate_id: id.clone(),
                site_cell_id: input.site_cell_id,
                euler_axis_cell_id: input.euler_axis_cell_id,
                euler_rate_microdegrees_per_million_years: input
                    .euler_rate_microdegrees_per_million_years,
                area_trillionths,
            }
        })
        .collect::<Vec<_>>();
    plates.sort_by(|left, right| left.plate_id.cmp(&right.plate_id));
    let positions = plates
        .iter()
        .enumerate()
        .map(|(index, plate)| {
            (
                plate.plate_id.clone(),
                u16::try_from(index).unwrap_or(u16::MAX),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let remapped = owners
        .iter()
        .map(|owner| positions[&original_ids[usize::from(*owner)]])
        .collect();
    let mut world = TectonicWorld {
        world_id: identity.world_id,
        generation_identity: identity.generation_identity,
        level: TECTONIC_LEVEL,
        generation_attempt: 1,
        construction_mode: ConstructionMode::FixedTest,
        plates,
        owners: remapped,
        boundary_edges: Vec::new(),
        boundary_components: Vec::new(),
        kinematic_samples: Vec::new(),
    };
    populate_boundaries_and_motion(&mut world, spec.radius_metres)?;
    Ok(world)
}

fn build_plates(
    spec: &WorldSpec,
    random: &NamedRandom,
    axis_candidates: &[CellId],
    sites: Vec<CellId>,
    plate_ids: Vec<PlateId>,
    areas: &[f64],
) -> Result<Vec<Plate>, TectonicError> {
    sites
        .into_iter()
        .zip(plate_ids)
        .zip(areas.iter().copied())
        .map(|((site, id), area)| {
            let axis_sample = random.bounded(
                "tectonics",
                &id.0,
                "euler-axis-cell",
                0,
                u64::try_from(axis_candidates.len()).unwrap_or(u64::MAX),
            )?;
            let axis_index = usize::try_from(axis_sample.value).unwrap_or(usize::MAX);
            let axis = *axis_candidates.get(axis_index).ok_or_else(|| {
                TectonicError::InvalidTopology("Euler axis sample is out of range".into())
            })?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let area_trillionths = (area / (4.0 * PI) * 1.0e12).round() as u64;
            Ok(Plate {
                euler_rate_microdegrees_per_million_years: spec.euler_rate(&id.0)?,
                plate_id: id,
                site_cell_id: site,
                euler_axis_cell_id: axis,
                area_trillionths,
            })
        })
        .collect()
}

fn select_sites(
    random: &NamedRandom,
    candidates: &[CellId],
    count: usize,
    attempt: u8,
) -> Result<Vec<CellId>, TectonicError> {
    let sample = random.bounded(
        "tectonics",
        "world",
        "first-plate-site",
        u64::from(attempt),
        u64::try_from(candidates.len()).unwrap_or(u64::MAX),
    )?;
    let first_index = usize::try_from(sample.value).unwrap_or(usize::MAX);
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
                .map(|site| quantized_dot(candidate.center_unit(), site.center_unit()))
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

fn site_separation_valid(sites: &[CellId]) -> bool {
    let maximum_dot = (20.0_f64.to_radians()).cos();
    sites.iter().enumerate().all(|(index, left)| {
        sites[index + 1..]
            .iter()
            .all(|right| left.center_unit().dot(right.center_unit()) <= maximum_dot)
    })
}

fn assign_owners(cells: &[CellId], sites: &[CellId], plate_ids: &[PlateId]) -> Vec<u16> {
    let site_vectors = sites
        .iter()
        .map(|site| site.center_unit())
        .collect::<Vec<_>>();
    cells
        .iter()
        .map(|cell| {
            let center = cell.center_unit();
            let mut best = (i64::MIN, &plate_ids[0], 0_usize);
            for (index, site) in site_vectors.iter().enumerate() {
                let candidate = (quantized_dot(center, *site), &plate_ids[index], index);
                if candidate.0 > best.0 || (candidate.0 == best.0 && candidate.1 < best.1) {
                    best = candidate;
                }
            }
            u16::try_from(best.2).unwrap_or(u16::MAX)
        })
        .collect()
}

fn plate_areas(cells: &[CellId], owners: &[u16], count: usize) -> Vec<f64> {
    let mut areas = vec![0.0; count];
    for (cell, owner) in cells.iter().zip(owners) {
        areas[usize::from(*owner)] += cell.unit_sphere_area();
    }
    areas
}

fn validate_domains(level: u8, owners: &[u16], areas: &[f64], count: usize) -> Result<(), String> {
    let total: f64 = areas.iter().sum();
    if ((total - 4.0 * PI) / (4.0 * PI)).abs() > 1.0e-12 {
        return Err("global plate area does not close".into());
    }
    if areas
        .iter()
        .any(|area| !(0.02..=0.35).contains(&(area / (4.0 * PI))))
    {
        return Err("plate area lies outside 2%..35%".into());
    }
    for plate in 0..count {
        let plate = u16::try_from(plate).unwrap_or(u16::MAX);
        if !owner_region_connected(level, owners, plate, false)? {
            return Err(format!("plate {plate} is disconnected"));
        }
        if !owner_region_connected(level, owners, plate, true)? {
            return Err(format!("plate {plate} contains a topological hole"));
        }
    }
    Ok(())
}

fn owner_region_connected(
    level: u8,
    owners: &[u16],
    plate: u16,
    complement: bool,
) -> Result<bool, String> {
    let belongs = |owner: u16| (owner == plate) != complement;
    let Some(start) = owners.iter().position(|owner| belongs(*owner)) else {
        return Ok(false);
    };
    let mut visited = vec![false; owners.len()];
    visited[start] = true;
    let mut queue = VecDeque::from([start]);
    let mut reached = 0_usize;
    while let Some(index) = queue.pop_front() {
        reached += 1;
        let cell = cell_from_index(level, index).map_err(|error| error.to_string())?;
        for neighbour in cell.neighbours().map_err(|error| error.to_string())? {
            let next = cell_index(neighbour);
            if !visited[next] && belongs(owners[next]) {
                visited[next] = true;
                queue.push_back(next);
            }
        }
    }
    Ok(reached == owners.iter().filter(|owner| belongs(**owner)).count())
}

fn populate_boundaries_and_motion(
    world: &mut TectonicWorld,
    radius_metres: u64,
) -> Result<(), TectonicError> {
    let cells = cells_at_level(world.level)?;
    let mut edges = Vec::new();
    for (index, cell) in cells.iter().copied().enumerate() {
        for direction in CardinalDirection::ALL {
            let neighbour = cell.neighbour(direction)?;
            if cell >= neighbour {
                continue;
            }
            let neighbour_index = cell_index(neighbour);
            let left = world.owners[index];
            let right = world.owners[neighbour_index];
            if left == right {
                continue;
            }
            let (a, b, cell_a, cell_b) = if left < right {
                (left, right, cell, neighbour)
            } else {
                (right, left, neighbour, cell)
            };
            let relation = cell.neighbour_relation(direction)?;
            let corners = edge_corners(direction);
            let mut vertices = [cell.vertex_id(corners[0])?, cell.vertex_id(corners[1])?];
            vertices.sort();
            edges.push(BoundaryEdge {
                edge_id: relation.shared_edge_id,
                plate_a: world.plates[usize::from(a)].plate_id.clone(),
                plate_b: world.plates[usize::from(b)].plate_id.clone(),
                cell_a,
                cell_b,
                vertices,
                component_index: u32::MAX,
            });
        }
    }
    edges.sort_by_key(|edge| edge.edge_id);
    let (components, vertex_degrees) = boundary_components(&world.world_id, &mut edges)?;
    if let Some((_, degree)) = vertex_degrees
        .iter()
        .find(|(_, degree)| **degree == 1 || **degree >= 4)
    {
        return Err(TectonicError::InvalidTopology(format!(
            "boundary vertex has unsupported degree {degree}"
        )));
    }
    let mut samples = Vec::with_capacity(edges.len());
    for edge in &edges {
        let component = components
            .get(usize::try_from(edge.component_index).unwrap_or(usize::MAX))
            .ok_or_else(|| TectonicError::InvalidTopology("edge component is missing".into()))?;
        samples.push(kinematic_sample(
            edge,
            component,
            &world.plates,
            radius_metres,
        )?);
    }
    world.boundary_edges = edges;
    world.boundary_components = components;
    world.kinematic_samples = samples;
    Ok(())
}

fn boundary_components(
    world_id: &str,
    edges: &mut [BoundaryEdge],
) -> Result<(Vec<BoundaryComponent>, BTreeMap<VertexId, usize>), TectonicError> {
    let mut vertex_edges: BTreeMap<VertexId, Vec<usize>> = BTreeMap::new();
    let mut groups: BTreeMap<(PlateId, PlateId), Vec<usize>> = BTreeMap::new();
    for (index, edge) in edges.iter().enumerate() {
        for vertex in &edge.vertices {
            vertex_edges.entry(vertex.clone()).or_default().push(index);
        }
        groups
            .entry((edge.plate_a.clone(), edge.plate_b.clone()))
            .or_default()
            .push(index);
    }
    let vertex_degrees = vertex_edges
        .iter()
        .map(|(vertex, incident)| (vertex.clone(), incident.len()))
        .collect::<BTreeMap<_, _>>();
    let mut components = Vec::new();
    for ((plate_a, plate_b), group) in groups {
        let allowed = group.iter().copied().collect::<BTreeSet<_>>();
        let mut unseen = allowed.clone();
        while let Some(start) = unseen.pop_first() {
            let mut queue = VecDeque::from([start]);
            let mut indices = Vec::new();
            while let Some(index) = queue.pop_front() {
                indices.push(index);
                for vertex in &edges[index].vertices {
                    let same_pair = vertex_edges[vertex]
                        .iter()
                        .copied()
                        .filter(|candidate| allowed.contains(candidate))
                        .collect::<Vec<_>>();
                    if same_pair.len() > 2 {
                        return Err(TectonicError::InvalidTopology(
                            "one plate-pair component branches".into(),
                        ));
                    }
                    for candidate in same_pair {
                        if unseen.remove(&candidate) {
                            queue.push_back(candidate);
                        }
                    }
                }
            }
            indices.sort_unstable();
            let minimum_edge_id = indices
                .iter()
                .map(|index| edges[*index].edge_id)
                .min()
                .ok_or_else(|| TectonicError::InvalidTopology("empty boundary component".into()))?;
            let identity_bytes = canonical_json(&(world_id, &plate_a, &plate_b, minimum_edge_id))?;
            let boundary_id =
                BoundaryId(format!("boundary-{}", &sha256_hex(&identity_bytes)[..24]));
            let component_index = u32::try_from(components.len()).unwrap_or(u32::MAX);
            for index in &indices {
                edges[*index].component_index = component_index;
            }
            let endpoints = indices
                .iter()
                .flat_map(|index| edges[*index].vertices.iter().cloned())
                .fold(BTreeMap::<VertexId, usize>::new(), |mut counts, vertex| {
                    *counts.entry(vertex).or_default() += 1;
                    counts
                });
            let is_loop = endpoints.values().all(|degree| *degree == 2);
            if !is_loop && endpoints.values().filter(|degree| **degree == 1).count() != 2 {
                return Err(TectonicError::InvalidTopology(
                    "boundary component is neither a loop nor a two-ended path".into(),
                ));
            }
            components.push(BoundaryComponent {
                boundary_id,
                plate_a: plate_a.clone(),
                plate_b: plate_b.clone(),
                minimum_edge_id,
                edge_indices: indices
                    .into_iter()
                    .map(|index| u32::try_from(index).unwrap_or(u32::MAX))
                    .collect(),
                is_loop,
            });
        }
    }
    components.sort_by(|left, right| left.boundary_id.cmp(&right.boundary_id));
    for (component_index, component) in components.iter().enumerate() {
        for edge_index in &component.edge_indices {
            edges[usize::try_from(*edge_index).unwrap_or(usize::MAX)].component_index =
                u32::try_from(component_index).unwrap_or(u32::MAX);
        }
    }
    Ok((components, vertex_degrees))
}

fn kinematic_sample(
    edge: &BoundaryEdge,
    component: &BoundaryComponent,
    plates: &[Plate],
    radius_metres: u64,
) -> Result<BoundaryKinematicSample, TectonicError> {
    let plate_a = plates
        .iter()
        .find(|plate| plate.plate_id == edge.plate_a)
        .ok_or_else(|| TectonicError::InvalidTopology("plate A is missing".into()))?;
    let plate_b = plates
        .iter()
        .find(|plate| plate.plate_id == edge.plate_b)
        .ok_or_else(|| TectonicError::InvalidTopology("plate B is missing".into()))?;
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
    let r = UnitVec3::new(endpoints[0].as_vec3() + endpoints[1].as_vec3())?;
    let delta = edge.cell_b.center() - edge.cell_a.center();
    let radial = r.as_vec3() * delta.dot(r.as_vec3());
    let n_ab = UnitVec3::new(delta - radial)?;
    let tangent = UnitVec3::new(n_ab.cross(r))?;
    let relative =
        plate_velocity(plate_b, r, radius_metres) - plate_velocity(plate_a, r, radius_metres);
    #[allow(clippy::cast_possible_truncation)]
    let normal = relative.dot(n_ab.as_vec3()).round() as i64;
    #[allow(clippy::cast_possible_truncation)]
    let tangent_rate = relative.dot(tangent.as_vec3()).round() as i64;
    Ok(BoundaryKinematicSample {
        edge_id: edge.edge_id,
        boundary_id: component.boundary_id.clone(),
        plate_a: edge.plate_a.clone(),
        plate_b: edge.plate_b.clone(),
        normal_rate_micrometres_per_year: normal,
        tangent_rate_micrometres_per_year: tangent_rate,
        tangent_significant: tangent_rate.abs() > RATE_DEADBAND_MICROMETRES_PER_YEAR,
        qualification: classify(normal, tangent_rate),
    })
}

fn plate_velocity(plate: &Plate, position: UnitVec3, radius_metres: u64) -> Vec3 {
    let axis = plate.euler_axis_cell_id.center_unit();
    #[allow(clippy::cast_precision_loss)]
    let scale =
        radius_metres as f64 * f64::from(plate.euler_rate_microdegrees_per_million_years) * PI
            / 180.0
            * 1.0e-6;
    axis.cross(position) * scale
}

#[must_use]
pub const fn classify(normal: i64, tangent: i64) -> BoundaryQualification {
    if normal < -RATE_DEADBAND_MICROMETRES_PER_YEAR {
        BoundaryQualification::Convergent
    } else if normal > RATE_DEADBAND_MICROMETRES_PER_YEAR {
        BoundaryQualification::Divergent
    } else if normal >= -RATE_DEADBAND_MICROMETRES_PER_YEAR
        && normal <= RATE_DEADBAND_MICROMETRES_PER_YEAR
        && tangent >= -RATE_DEADBAND_MICROMETRES_PER_YEAR
        && tangent <= RATE_DEADBAND_MICROMETRES_PER_YEAR
    {
        BoundaryQualification::Stable
    } else {
        BoundaryQualification::Unresolved
    }
}

impl TectonicWorld {
    /// Returns a materialized owner.
    ///
    /// # Errors
    /// Rejects cells outside this field or malformed owner indices.
    pub fn owner_of_cell(&self, cell: CellId) -> Result<&Plate, TectonicError> {
        if cell.level() != self.level {
            return Err(TectonicError::CellOutsideField(cell));
        }
        let index = cell_index(cell);
        let owner = *self
            .owners
            .get(index)
            .ok_or(TectonicError::CellOutsideField(cell))?;
        self.plates
            .get(usize::from(owner))
            .ok_or_else(|| TectonicError::InvalidTopology("owner index is invalid".into()))
    }

    /// Returns the continuous nearest-site owner for any sphere direction.
    ///
    /// # Errors
    /// Rejects invalid vectors.
    pub fn owner_at(&self, direction: Vec3) -> Result<&Plate, TectonicError> {
        let point = UnitVec3::new(direction)?;
        self.plates
            .iter()
            .max_by(|left, right| {
                let left_dot = quantized_dot(point, left.site_cell_id.center_unit());
                let right_dot = quantized_dot(point, right.site_cell_id.center_unit());
                left_dot
                    .cmp(&right_dot)
                    .then_with(|| right.plate_id.cmp(&left.plate_id))
            })
            .ok_or_else(|| TectonicError::InvalidTopology("world has no plates".into()))
    }

    #[must_use]
    pub fn plate(&self, plate_id: &PlateId) -> Option<&Plate> {
        self.plates
            .binary_search_by(|plate| plate.plate_id.cmp(plate_id))
            .ok()
            .map(|index| &self.plates[index])
    }

    #[must_use]
    pub fn boundary(&self, boundary_id: &BoundaryId) -> Option<&BoundaryComponent> {
        self.boundary_components
            .binary_search_by(|boundary| boundary.boundary_id.cmp(boundary_id))
            .ok()
            .map(|index| &self.boundary_components[index])
    }

    #[must_use]
    pub fn discrete_digest(&self) -> String {
        let bytes = canonical_json(self).unwrap_or_default();
        sha256_hex(&bytes)
    }
}

fn plate_id(world_id: &str, site: CellId) -> Result<PlateId, TectonicError> {
    let bytes = canonical_json(&(world_id, site))?;
    Ok(PlateId(format!("plate-{}", &sha256_hex(&bytes)[..24])))
}

fn fixed_plate_id(world_id: &str, plate_key: &str) -> Result<PlateId, TectonicError> {
    let bytes = canonical_json(&(world_id, "fixed-test", plate_key))?;
    Ok(PlateId(format!("plate-{}", &sha256_hex(&bytes)[..24])))
}

fn quantized_dot(left: UnitVec3, right: UnitVec3) -> i64 {
    #[allow(clippy::cast_possible_truncation)]
    let value = (left.dot(right) * 1.0e15).round() as i64;
    value
}

fn cells_at_level(level: u8) -> Result<Vec<CellId>, SphereError> {
    let count = 1_u32 << level;
    let capacity = 6_usize * usize::try_from(count).unwrap_or(usize::MAX).pow(2);
    let mut cells = Vec::with_capacity(capacity);
    for face in CubeFace::ALL {
        for j in 0..count {
            for i in 0..count {
                cells.push(CellId::new(face, level, i, j)?);
            }
        }
    }
    Ok(cells)
}

fn cell_index(cell: CellId) -> usize {
    let count = 1_usize << cell.level();
    let face = cell.face() as usize;
    let i = usize::try_from(cell.i()).unwrap_or(usize::MAX);
    let j = usize::try_from(cell.j()).unwrap_or(usize::MAX);
    face * count * count + j * count + i
}

fn cell_from_index(level: u8, index: usize) -> Result<CellId, SphereError> {
    let count = 1_usize << level;
    let face_size = count * count;
    let face_index = index / face_size;
    let local = index % face_size;
    let face = *CubeFace::ALL
        .get(face_index)
        .ok_or(SphereError::InvalidCell)?;
    CellId::new(
        face,
        level,
        u32::try_from(local % count).map_err(|_| SphereError::InvalidCell)?,
        u32::try_from(local / count).map_err(|_| SphereError::InvalidCell)?,
    )
}

const fn edge_corners(direction: CardinalDirection) -> [u8; 2] {
    match direction {
        CardinalDirection::West => [0, 3],
        CardinalDirection::East => [1, 2],
        CardinalDirection::South => [0, 1],
        CardinalDirection::North => [3, 2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sphere_core::{cell_at, cube_rotations};

    #[test]
    fn classification_respects_the_integer_deadband() {
        assert_eq!(classify(-5_001, 0), BoundaryQualification::Convergent);
        assert_eq!(classify(5_001, 0), BoundaryQualification::Divergent);
        assert_eq!(classify(5_000, -5_000), BoundaryQualification::Stable);
        assert_eq!(classify(0, 5_001), BoundaryQualification::Unresolved);
    }

    #[test]
    fn small_partition_is_complete_connected_and_queryable() {
        let spec = WorldSpec::earthlike("E2-small-partition").unwrap();
        let world = build_at_level(&spec, 4).unwrap();
        assert_eq!(world.owners.len(), 6 * 16 * 16);
        assert!((8..=10).contains(&world.plates.len()));
        let cell = CellId::new(CubeFace::PosX, 4, 3, 9).unwrap();
        assert_eq!(
            world.owner_of_cell(cell).unwrap(),
            world.owner_at(cell.center()).unwrap()
        );
        assert!(!world.boundary_edges.is_empty());
        assert_eq!(world.boundary_edges.len(), world.kinematic_samples.len());
    }

    #[test]
    fn pv01_uses_fixed_six_plate_input() {
        let spec = WorldSpec::earthlike("PV01-E2-v1").unwrap();
        let world = build_fixed(&spec, &pv01_fixed_input().unwrap()).unwrap();
        assert_eq!(world.construction_mode, ConstructionMode::FixedTest);
        assert_eq!(world.plates.len(), 6);
        assert!(
            world
                .kinematic_samples
                .iter()
                .any(|sample| sample.qualification == BoundaryQualification::Convergent)
        );
        assert!(
            world
                .kinematic_samples
                .iter()
                .any(|sample| sample.qualification == BoundaryQualification::Divergent)
        );
        for (left, right, expected, normal) in [
            (
                "cs1/F2/L8/31111103",
                "cs1/F2/L8/31111112",
                BoundaryQualification::Convergent,
                -133_423,
            ),
            (
                "cs1/F5/L8/33333321",
                "cs1/F5/L8/33333323",
                BoundaryQualification::Divergent,
                86_233,
            ),
            (
                "cs1/F0/L8/11111101",
                "cs1/F0/L8/11111110",
                BoundaryQualification::Stable,
                0,
            ),
        ] {
            let left: CellId = left.parse().unwrap();
            let right: CellId = right.parse().unwrap();
            let sample = world
                .kinematic_samples
                .iter()
                .find(|sample| sample.edge_id.cells() == [left, right])
                .unwrap();
            assert_eq!(sample.qualification, expected);
            assert_eq!(sample.normal_rate_micrometres_per_year, normal);
        }
        validate(&world, 6_371_000).unwrap();
    }

    #[test]
    fn pv01_input_order_and_six_rotations_preserve_physical_relations() {
        let spec = WorldSpec::earthlike("PV01-E2-v1").unwrap();
        let fixed = pv01_fixed_input().unwrap();
        let baseline = build_fixed(&spec, &fixed).unwrap();

        let mut reversed = fixed.clone();
        reversed.reverse();
        assert_eq!(baseline, build_fixed(&spec, &reversed).unwrap());

        let baseline_areas = baseline
            .plates
            .iter()
            .map(|plate| plate.area_trillionths)
            .collect::<BTreeSet<_>>();
        let baseline_qualifications =
            baseline
                .kinematic_samples
                .iter()
                .fold(BTreeMap::new(), |mut counts, sample| {
                    *counts
                        .entry(format!("{:?}", sample.qualification))
                        .or_insert(0_usize) += 1;
                    counts
                });
        for rotation in cube_rotations().into_iter().take(6) {
            let rotated = fixed
                .iter()
                .map(|plate| {
                    Ok(FixedPlateInput {
                        plate_key: plate.plate_key.clone(),
                        site_cell_id: cell_at(
                            rotation.apply(plate.site_cell_id.center()),
                            PLATE_SITE_LEVEL,
                        )?,
                        euler_axis_cell_id: cell_at(
                            rotation.apply(plate.euler_axis_cell_id.center()),
                            PLATE_SITE_LEVEL,
                        )?,
                        euler_rate_microdegrees_per_million_years: plate
                            .euler_rate_microdegrees_per_million_years,
                    })
                })
                .collect::<Result<Vec<_>, SphereError>>()
                .unwrap();
            let world = build_fixed(&spec, &rotated).unwrap();
            let areas = world
                .plates
                .iter()
                .map(|plate| plate.area_trillionths)
                .collect::<BTreeSet<_>>();
            let qualifications =
                world
                    .kinematic_samples
                    .iter()
                    .fold(BTreeMap::new(), |mut counts, sample| {
                        *counts
                            .entry(format!("{:?}", sample.qualification))
                            .or_insert(0_usize) += 1;
                        counts
                    });
            assert_eq!(areas, baseline_areas);
            assert_eq!(world.boundary_edges.len(), baseline.boundary_edges.len());
            assert_eq!(qualifications, baseline_qualifications);
        }
    }
}
