import hashlib
import json
from pathlib import Path
import unittest

from oracle import (
    classify_boundary,
    div_round_half_even,
    cubed_sphere_cell_area,
    cubed_sphere_global_area,
    equiangular_face_vector,
    euler_velocity_micrometres_per_year,
    named_random_u64,
    normalize_seed,
    linear_ramp_capacity,
    map_u53_to_range,
    unit_sphere_area,
)


class OracleTests(unittest.TestCase):
    def test_half_ramp(self) -> None:
        result = linear_ramp_capacity(0.5)
        self.assertAlmostEqual(result.area, 0.125, places=15)
        self.assertAlmostEqual(result.volume, 1.0 / 48.0, places=15)

    def test_full_ramp(self) -> None:
        result = linear_ramp_capacity(2.0)
        self.assertAlmostEqual(result.area, 0.5, places=15)
        self.assertAlmostEqual(result.volume, 2.0 / 3.0, places=15)

    def test_unit_sphere_area(self) -> None:
        self.assertGreater(unit_sphere_area(), 12.0)

    def test_equiangular_vectors_are_unit_length(self) -> None:
        for face in range(6):
            vector = equiangular_face_vector(face, 0.37, -0.61)
            self.assertAlmostEqual(sum(value * value for value in vector), 1.0, places=14)

    def test_cubed_sphere_area_closes_independently(self) -> None:
        self.assertLess(abs(cubed_sphere_global_area(5) - unit_sphere_area()), 1e-12)

    def test_child_areas_close_to_parent(self) -> None:
        parent = cubed_sphere_cell_area(4, 3, 5, 2)
        children = sum(
            cubed_sphere_cell_area(4, 4, 10 + di, 4 + dj)
            for dj in range(2)
            for di in range(2)
        )
        self.assertLess(abs(children - parent) / parent, 1e-12)

    def test_e2_seed_normalization(self) -> None:
        self.assertEqual(normalize_seed("  e\u0301  world "), "é world")
        with self.assertRaises(ValueError):
            normalize_seed("line\nbreak")

    def test_e2_named_random_fixed_vector(self) -> None:
        value = named_random_u64(
            "ec2a01a43513f16acb9dd774655b616521be87b6d1cd8cca57d2387ba9a6b15f",
            "tectonics",
            "world",
            "plate-count-quantile",
            0,
        )
        self.assertEqual(value, 14770547165756481792)
        self.assertEqual(value >> 11, 7212181233279532)

    def test_e2_euler_velocity_analytic_vector(self) -> None:
        velocity = euler_velocity_micrometres_per_year(
            (0.0, 0.0, 1.0), (1.0, 0.0, 0.0), 6_371_000, 1_000_000
        )
        self.assertAlmostEqual(velocity[0], 0.0, places=12)
        self.assertAlmostEqual(velocity[1], 111194.92664455874, places=9)
        self.assertAlmostEqual(velocity[2], 0.0, places=12)

    def test_e2_integer_boundary_classification(self) -> None:
        self.assertEqual(classify_boundary(-5001, 0), "CONVERGENT")
        self.assertEqual(classify_boundary(5001, 0), "DIVERGENT")
        self.assertEqual(classify_boundary(5000, -5000), "STABLE")
        self.assertEqual(classify_boundary(0, 5001), "UNRESOLVED")

    def test_e21_half_even_integer_vectors(self) -> None:
        self.assertEqual(div_round_half_even(5, 2), 2)
        self.assertEqual(div_round_half_even(7, 2), 4)
        self.assertEqual(div_round_half_even(-5, 2), -2)
        self.assertEqual(div_round_half_even(-7, 2), -4)
        with self.assertRaises(ValueError):
            div_round_half_even(1, 0)

    def test_e21_u53_range_vectors(self) -> None:
        denominator = 1 << 53
        self.assertEqual(map_u53_to_range(0, 200_000, 700_001), 200_000)
        self.assertEqual(
            map_u53_to_range(denominator - 1, 200_000, 700_001), 700_000
        )

    def test_e21_legacy_baseline_protects_frozen_assets(self) -> None:
        engine_root = Path(__file__).resolve().parents[3]
        baseline_path = engine_root / "fixtures/e2.1/baselines/e2-v1-freeze.json"
        baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
        self.assertEqual(baseline["legacy_status"], "FROZEN_LEGACY_PROTOTYPE")
        self.assertEqual(
            baseline["source_tree_sha256"],
            "94fd53c3f05c2e26827e357a5e08f4c9e5a8d0908425c643572987cd9c5069d1",
        )
        for relative, expected in baseline["protected_files"].items():
            actual = hashlib.sha256((engine_root / relative).read_bytes()).hexdigest()
            self.assertEqual(actual, expected, relative)

    def test_e21_schema_skeletons_are_self_describing(self) -> None:
        engine_root = Path(__file__).resolve().parents[3]
        schema_manifest = json.loads(
            (engine_root / "schemas/e2.1-manifest.json").read_text(encoding="utf-8")
        )
        self.assertEqual(schema_manifest["status"], "FROZEN")
        self.assertEqual(len(schema_manifest["schemas"]), 7)
        for name in schema_manifest["schemas"]:
            schema = json.loads((engine_root / "schemas" / name).read_text(encoding="utf-8"))
            self.assertEqual(schema["$schema"], "https://json-schema.org/draft/2020-12/schema")
            self.assertTrue(schema["$id"].startswith("planet-engine/"))
            self.assertEqual(schema["type"], "object")
            self.assertGreater(len(schema["required"]), 0)


if __name__ == "__main__":
    unittest.main()
