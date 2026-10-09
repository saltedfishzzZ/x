use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use tectonics_core::v2::{
    ExecutionConfigV2, ShardOrderV2, TECTONIC_SHARD_COUNT_V2, build_natural_skeleton_with_execution,
};
use world_contracts::{canonical_json, sha256_hex};
use world_spec::v2::WorldIntentSpecV2;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeedSet {
    fixture_id: String,
    schema: String,
    purpose: String,
    seeds: Vec<String>,
}

#[derive(Serialize)]
struct SeedEvidence {
    seed: String,
    world_id: String,
    tectonic_skeleton_id: String,
    plate_count: usize,
    owners_sha256: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let fixture_path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: e21_seed_suite <fixture.json>")?;
    let fixture: SeedSet = serde_json::from_slice(&fs::read(&fixture_path)?)?;
    if fixture.schema != "E2.1-SEED-SET-v2" {
        return Err(format!("unsupported seed-set schema: {}", fixture.schema).into());
    }
    if fixture.seeds.len() != 32
        || fixture.seeds.iter().collect::<BTreeSet<_>>().len() != fixture.seeds.len()
    {
        return Err("seed set must contain exactly 32 unique seeds".into());
    }

    let mut worlds = Vec::with_capacity(fixture.seeds.len());
    for seed in fixture.seeds {
        let intent = WorldIntentSpecV2::earthlike_natural(&seed)?;
        let identity = intent.identity()?;
        let (skeleton, execution) = build_natural_skeleton_with_execution(
            &intent,
            ExecutionConfigV2 {
                worker_count: 2,
                shard_order: ShardOrderV2::Forward,
            },
        )?;
        if execution.participating_worker_count != 2
            || execution.shard_count != TECTONIC_SHARD_COUNT_V2
        {
            return Err(format!("invalid execution evidence for {seed}").into());
        }
        worlds.push(SeedEvidence {
            seed,
            world_id: identity.world_id,
            tectonic_skeleton_id: skeleton.tectonic_skeleton_id.0,
            plate_count: skeleton.plate_roles.len(),
            owners_sha256: sha256_hex(&canonical_json(&skeleton.owners)?),
        });
    }

    let evidence_sha256 = sha256_hex(&canonical_json(&worlds)?);
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "schema": "e2.1-seed-suite-evidence-v1",
            "fixture_id": fixture.fixture_id,
            "purpose": fixture.purpose,
            "world_count": worlds.len(),
            "worker_count": 2,
            "shard_count": TECTONIC_SHARD_COUNT_V2,
            "worlds": worlds,
            "evidence_sha256": evidence_sha256,
            "pass": true
        }))?
    );
    Ok(())
}
