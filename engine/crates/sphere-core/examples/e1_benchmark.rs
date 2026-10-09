use sphere_core::{
    CellId, CubeFace, FaceCoord, PatchPlan, PatchRequest, SpatialReference, cell_at, face_to_vector,
};
use std::hint::black_box;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let reference = SpatialReference::prototype()?;

    let geometry_started = Instant::now();
    let mut area_checksum = 0.0;
    let count = 1_u32 << 8;
    for face in CubeFace::ALL {
        for j in 0..count {
            for i in 0..count {
                area_checksum += CellId::new(face, 8, i, j)?
                    .geometry(&reference)?
                    .unit_sphere_area;
            }
        }
    }
    black_box(area_checksum);
    let geometry_seconds = geometry_started.elapsed().as_secs_f64();

    let locate_started = Instant::now();
    let mut identity_checksum = 0_u64;
    for index in 0_u32..100_000 {
        let face = CubeFace::ALL[usize::try_from(index % 6)?];
        let s = -0.999 + 1.998 * f64::from(index % 997) / 996.0;
        let t = -0.999 + 1.998 * f64::from((index * 37) % 991) / 990.0;
        identity_checksum ^= cell_at(face_to_vector(FaceCoord { face, s, t }), 10)?.morton_path();
    }
    black_box(identity_checksum);
    let locate_seconds = locate_started.elapsed().as_secs_f64();

    let patch_started = Instant::now();
    let patch = PatchPlan::build(PatchRequest {
        patch_id: "E1F06".into(),
        core_parents: vec![CellId::new(CubeFace::PosX, 8, 120, 130)?],
        target_level: 10,
        halo_parent_rings: 1,
        cell_budget: 128,
    })?;
    black_box(patch.discrete_signature());
    let patch_seconds = patch_started.elapsed().as_secs_f64();

    println!(
        "{}",
        serde_json::json!({
            "profile": "E1-PERFORMANCE-v1",
            "geometry_l8": {
                "cell_count": 393_216,
                "seconds": geometry_seconds,
                "limit_seconds": 10.0,
                "pass": geometry_seconds <= 10.0
            },
            "locate_l10": {
                "query_count": 100_000,
                "seconds": locate_seconds,
                "limit_seconds": 3.0,
                "pass": locate_seconds <= 3.0
            },
            "patch_e1f06": {
                "seconds": patch_seconds,
                "limit_seconds": 2.0,
                "pass": patch_seconds <= 2.0
            }
        })
    );
    Ok(())
}
