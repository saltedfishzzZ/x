use sphere_core::{
    CardinalDirection, CellId, CoverageRole, CubeFace, FaceCoord, PatchPlan, PatchRequest,
    SpatialReference, SphereError, SphereQuery, SphericalCap, TangentFrame, UnitVec3, Vec3,
    cell_at, cube_rotations, face_to_unit_vector, global_unit_area, unit_sphere_area_error,
    vector_to_face,
};
use std::collections::{BTreeSet, HashSet};
use std::f64::consts::PI;

fn cell(face: CubeFace, level: u8, i: u32, j: u32) -> CellId {
    CellId::new(face, level, i, j).unwrap()
}

#[test]
fn e1_g01_identity_round_trip_parent_child_and_no_sample_collision() {
    let mut identities = HashSet::new();
    for level in 0..=30 {
        let count = 1_u32 << level;
        let indices = BTreeSet::from([0, count / 3, count / 2, count - 1]);
        for face in CubeFace::ALL {
            for &i in &indices {
                for &j in &indices {
                    let current = cell(face, level, i, j);
                    assert_eq!(current.to_string().parse(), Ok(current));
                    assert!(identities.insert(current.to_string()));
                    if let Some(parent) = current.parent() {
                        assert!(parent.children().unwrap().contains(&current));
                    }
                }
            }
        }
    }
}

#[test]
fn e1_g01_child_order_is_southwest_southeast_northwest_northeast() {
    let root = cell(CubeFace::PosX, 0, 0, 0);
    assert_eq!(
        root.children().unwrap(),
        [
            cell(CubeFace::PosX, 1, 0, 0),
            cell(CubeFace::PosX, 1, 1, 0),
            cell(CubeFace::PosX, 1, 0, 1),
            cell(CubeFace::PosX, 1, 1, 1),
        ]
    );
}

#[test]
fn e1_g02_all_l4_neighbours_are_reciprocal_with_shared_edges() {
    let count = 1_u32 << 4;
    for face in CubeFace::ALL {
        for j in 0..count {
            for i in 0..count {
                let source = cell(face, 4, i, j);
                for direction in CardinalDirection::ALL {
                    let relation = source.neighbour_relation(direction).unwrap();
                    let reverse = relation
                        .target_cell
                        .neighbour_relation(relation.target_edge)
                        .unwrap();
                    assert_eq!(reverse.target_cell, source);
                    assert_eq!(reverse.shared_edge_id, relation.shared_edge_id);
                }
            }
        }
    }
}

#[test]
fn e1_g02_cube_corner_has_three_incident_cells() {
    let corner = cell(CubeFace::PosX, 5, 31, 31).vertex_id(2).unwrap();
    assert_eq!(corner.incident_cells().len(), 3);
    assert_eq!(
        corner
            .incident_cells()
            .iter()
            .map(|entry| entry.face())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([CubeFace::PosX, CubeFace::PosY, CubeFace::PosZ])
    );
}

#[test]
fn e1_g03_unit_vector_contract_and_invalid_inputs() {
    for face in CubeFace::ALL {
        for s in [-1.0, -0.25, 0.0, 0.75, 1.0] {
            for t in [-1.0, -0.4, 0.0, 0.6, 1.0] {
                let vector = face_to_unit_vector(FaceCoord { face, s, t }).unwrap();
                assert!((vector.as_vec3().norm() - 1.0).abs() <= 5.0e-15);
            }
        }
    }
    assert_eq!(
        UnitVec3::new(Vec3::new(0.0, 0.0, 0.0)),
        Err(SphereError::InvalidVector)
    );
    assert_eq!(
        UnitVec3::new(Vec3::new(f64::NAN, 0.0, 0.0)),
        Err(SphereError::InvalidVector)
    );
}

#[test]
fn e1_g04_non_boundary_face_mapping_round_trip() {
    for face in CubeFace::ALL {
        for s in [-0.91, -0.31, 0.0, 0.27, 0.88] {
            for t in [-0.87, -0.21, 0.0, 0.39, 0.93] {
                let source = FaceCoord { face, s, t };
                let mapped = face_to_unit_vector(source).unwrap();
                let round_trip = vector_to_face(mapped.as_vec3()).unwrap();
                assert_eq!(round_trip.face, face);
                assert!((round_trip.s - s).abs() <= 2.0e-14);
                assert!((round_trip.t - t).abs() <= 2.0e-14);
            }
        }
    }
}

#[test]
fn e1_g04_ties_use_x_then_y_then_z() {
    assert_eq!(
        vector_to_face(Vec3::new(1.0, 1.0, 1.0)).unwrap().face,
        CubeFace::PosX
    );
    assert_eq!(
        vector_to_face(Vec3::new(0.0, -1.0, 1.0)).unwrap().face,
        CubeFace::NegY
    );
}

#[test]
fn e1_g05_l8_global_area_closes() {
    assert!(unit_sphere_area_error(8).unwrap() <= 1.0e-12);
    assert!((global_unit_area(8).unwrap() - 4.0 * PI).abs() <= 4.0 * PI * 1.0e-12);
}

#[test]
fn e1_g06_children_preserve_parent_area() {
    for parent in [
        cell(CubeFace::PosX, 0, 0, 0),
        cell(CubeFace::NegY, 3, 0, 7),
        cell(CubeFace::PosZ, 8, 127, 200),
    ] {
        let children_area: f64 = parent
            .children()
            .unwrap()
            .into_iter()
            .map(CellId::unit_sphere_area)
            .sum();
        let relative =
            (children_area - parent.unit_sphere_area()).abs() / parent.unit_sphere_area();
        assert!(relative <= 1.0e-12, "{parent}: {relative:e}");
    }
}

#[test]
fn e1_g07_tangent_frame_is_orthonormal_and_right_handed() {
    for normal in [
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(-0.5, 0.7, -0.2),
    ] {
        let frame = TangentFrame::at(UnitVec3::new(normal).unwrap()).unwrap();
        assert!(frame.normal.dot(frame.u_axis).abs() <= 2.0e-14);
        assert!(frame.normal.dot(frame.v_axis).abs() <= 2.0e-14);
        assert!(frame.u_axis.dot(frame.v_axis).abs() <= 2.0e-14);
        assert!(frame.u_axis.cross(frame.v_axis).dot(frame.normal.as_vec3()) > 1.0 - 2.0e-14);
    }
}

#[test]
fn e1_g08_cube_rotations_preserve_cell_area() {
    let source = cell(CubeFace::PosX, 5, 7, 19);
    assert_eq!(cube_rotations().len(), 24);
    for rotation in cube_rotations() {
        let rotated = cell_at(rotation.apply(source.center()), source.level()).unwrap();
        let relative = (rotated.unit_sphere_area() - source.unit_sphere_area()).abs()
            / source.unit_sphere_area();
        assert!(relative <= 2.0e-14);
        for neighbour in source.neighbours().unwrap() {
            let rotated_neighbour =
                cell_at(rotation.apply(neighbour.center()), neighbour.level()).unwrap();
            assert!(rotated.neighbours().unwrap().contains(&rotated_neighbour));
        }
    }
}

#[test]
fn e1_g09_cap_cover_is_sorted_unique_and_contains_sampled_points() {
    let query = SphereQuery::new(SpatialReference::prototype().unwrap(), None);
    let center = UnitVec3::new(Vec3::new(1.0, 0.2, -0.1)).unwrap();
    let cap = SphericalCap::new(center, 0.22).unwrap();
    let coverage = query.cover_cap(cap, 5).unwrap();
    assert!(coverage.windows(2).all(|pair| pair[0] < pair[1]));
    let coverage_set: BTreeSet<_> = coverage.iter().copied().collect();
    assert_eq!(coverage_set.len(), coverage.len());
    let frame = TangentFrame::at(center).unwrap();
    for radial_fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
        for angle_index in 0..48 {
            let angle = f64::from(angle_index) * 2.0 * PI / 48.0;
            let radius = cap.angular_radius * radial_fraction;
            let tangent =
                frame.u_axis.as_vec3() * angle.cos() + frame.v_axis.as_vec3() * angle.sin();
            let sample = center.as_vec3() * radius.cos() + tangent * radius.sin();
            assert!(coverage_set.contains(&cell_at(sample, 5).unwrap()));
        }
    }
}

#[test]
fn e1_g10_patch_has_three_scales_and_area_weights() {
    let parent = cell(CubeFace::PosX, 8, 120, 130);
    let patch = PatchPlan::build(PatchRequest {
        patch_id: "E1F06".into(),
        core_parents: vec![parent],
        target_level: 10,
        halo_parent_rings: 1,
        cell_budget: 128,
    })
    .unwrap();
    assert!(patch.core.iter().all(|entry| entry.level() == 10));
    assert!(
        patch
            .transition_leaves
            .iter()
            .all(|entry| entry.level() == 9)
    );
    assert!(patch.halo.iter().all(|entry| entry.level() == 10));
    assert!(
        patch
            .halo
            .iter()
            .all(|entry| patch.coverage_role(*entry) == CoverageRole::Halo)
    );
    let weights = patch.aggregation_weights(parent).unwrap();
    assert_eq!(weights.len(), 16);
    assert!(
        (weights
            .iter()
            .map(|entry| entry.true_area_weight)
            .sum::<f64>()
            - 1.0)
            .abs()
            <= 1.0e-12
    );
}

#[test]
fn e1_g11_patch_signature_is_input_order_independent() {
    let a = cell(CubeFace::PosX, 8, 120, 130);
    let b = cell(CubeFace::PosX, 8, 121, 130);
    let build = |parents| {
        PatchPlan::build(PatchRequest {
            patch_id: "stable".into(),
            core_parents: parents,
            target_level: 10,
            halo_parent_rings: 1,
            cell_budget: 256,
        })
        .unwrap()
        .discrete_signature()
    };
    let expected = build(vec![a, b]);
    assert_eq!(expected, build(vec![b, a, b]));
    for worker_count in [1, 2, 4] {
        let signatures = std::thread::scope(|scope| {
            (0..worker_count)
                .map(|index| {
                    let parents = if index % 2 == 0 {
                        vec![a, b]
                    } else {
                        vec![b, a, b]
                    };
                    scope.spawn(move || build(parents))
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(signatures.iter().all(|signature| signature == &expected));
    }
}

#[test]
fn e1_f08_rejects_invalid_levels_radii_caps_and_patch_budgets() {
    assert!(CellId::new(CubeFace::PosX, 31, 0, 0).is_err());
    assert!(SpatialReference::new(-1.0, 8, 10).is_err());
    let center = UnitVec3::new(Vec3::new(1.0, 0.0, 0.0)).unwrap();
    assert!(SphericalCap::new(center, PI.next_up()).is_err());
    let result = PatchPlan::build(PatchRequest {
        patch_id: "too-small".into(),
        core_parents: vec![cell(CubeFace::PosX, 8, 100, 100)],
        target_level: 10,
        halo_parent_rings: 1,
        cell_budget: 1,
    });
    assert!(matches!(
        result,
        Err(SphereError::PatchBudgetExceeded { .. })
    ));
}
