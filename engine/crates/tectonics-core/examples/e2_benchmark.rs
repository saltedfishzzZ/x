use serde::Deserialize;
use sphere_core::{CellId, CubeFace};
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;
use tectonics_core::build;
use world_contracts::{canonical_json, sha256_hex};
use world_spec::WorldSpec;

#[derive(Deserialize)]
struct SeedSet {
    seeds: Vec<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fixture: SeedSet =
        serde_json::from_slice(&fs::read(root.join("fixtures/tectonics/PVAL-E2-v1.json"))?)?;
    let started = Instant::now();
    let mut evidence = Vec::new();
    let mut maximum_attempt = 0_u8;
    let mut last_world = None;
    for seed in &fixture.seeds {
        let world = build(&WorldSpec::earthlike(seed)?)?;
        maximum_attempt = maximum_attempt.max(world.generation_attempt);
        evidence.push((seed, world.discrete_digest()));
        last_world = Some(world);
    }
    let statistical_seconds = started.elapsed().as_secs_f64();
    let world = last_world.ok_or("PVAL seed set is empty")?;

    let query_started = Instant::now();
    let mut owner_accumulator = 0_usize;
    for sample in 0..100_000_usize {
        let face = CubeFace::ALL[sample % 6];
        let i = u32::try_from((sample * 17) % 256)?;
        let j = u32::try_from((sample * 43) % 256)?;
        let cell = CellId::new(face, 8, i, j)?;
        owner_accumulator ^= world.owner_of_cell(cell)?.plate_id.0.len();
    }
    black_box(owner_accumulator);
    let owner_query_seconds = query_started.elapsed().as_secs_f64();

    let detail_started = Instant::now();
    let mut detail_accumulator = 0_usize;
    for sample in 0..10_000_usize {
        let plate = &world.plates[sample % world.plates.len()];
        detail_accumulator ^= world
            .plate(&plate.plate_id)
            .ok_or("plate query failed")?
            .plate_id
            .0
            .len();
        let boundary = &world.boundary_components[sample % world.boundary_components.len()];
        detail_accumulator ^= world
            .boundary(&boundary.boundary_id)
            .ok_or("boundary query failed")?
            .edge_indices
            .len();
    }
    black_box(detail_accumulator);
    let detail_query_seconds = detail_started.elapsed().as_secs_f64();

    let evidence_sha256 = sha256_hex(&canonical_json(&evidence)?);
    let estimated_bytes = world.owners.len() * size_of::<u16>()
        + world.boundary_edges.len() * size_of::<tectonics_core::BoundaryEdge>()
        + world.kinematic_samples.len() * size_of::<tectonics_core::BoundaryKinematicSample>();
    println!(
        "{}",
        serde_json::json!({
            "detail_queries": {
                "count": 10_000,
                "pass": detail_query_seconds < 2.0,
                "seconds": detail_query_seconds
            },
            "estimated_resident_upper_bound_bytes": estimated_bytes * 8,
            "evidence_sha256": evidence_sha256,
            "maximum_generation_attempt": maximum_attempt,
            "owner_queries": {
                "count": 100_000,
                "pass": owner_query_seconds < 3.0,
                "seconds": owner_query_seconds
            },
            "pval": {
                "count": fixture.seeds.len(),
                "pass": fixture.seeds.len() == 32 && statistical_seconds < 300.0,
                "seconds": statistical_seconds
            }
        })
    );
    Ok(())
}
