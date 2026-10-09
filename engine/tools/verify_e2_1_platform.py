#!/usr/bin/env python3
"""执行E2.1阶段门禁；G01至G17逐项取证，平台门禁留待原生双端验证。"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ENGINE_ROOT = Path(__file__).resolve().parents[1]
SHA256 = 64


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def run_check(
    name: str, command: list[str], cwd: Path = ENGINE_ROOT
) -> dict[str, Any]:
    started = time.monotonic()
    completed = subprocess.run(
        command,
        cwd=cwd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    output = completed.stdout + completed.stderr
    status = "PASS" if completed.returncode == 0 else "FAIL"
    print(f"[{status}] {name}")
    if status == "FAIL":
        print(output)
    return {
        "name": name,
        "status": status,
        "return_code": completed.returncode,
        "duration_seconds": round(time.monotonic() - started, 6),
        "output_sha256": digest_bytes(output.encode()),
        "output": output,
    }


def synthetic_check(name: str, errors: list[str], evidence: Any) -> dict[str, Any]:
    output = "\n".join(errors) if errors else json.dumps(evidence, ensure_ascii=False, sort_keys=True)
    status = "FAIL" if errors else "PASS"
    print(f"[{status}] {name}")
    return {
        "name": name,
        "status": status,
        "return_code": 1 if errors else 0,
        "duration_seconds": 0.0,
        "output_sha256": digest_bytes(output.encode()),
        "output": output,
    }


def last_json(check: dict[str, Any]) -> dict[str, Any] | None:
    for line in reversed(check["output"].splitlines()):
        if line.startswith("{"):
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                continue
    return None


def source_tree_sha256() -> str:
    files = [ENGINE_ROOT / name for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")]
    for name in ("crates", "fixtures", "reference/python", "schemas", "tools"):
        files.extend(path for path in (ENGINE_ROOT / name).rglob("*") if path.is_file())
    digest = hashlib.sha256()
    for path in sorted(set(files)):
        if "__pycache__" in path.parts or path.suffix in {".pyc", ".DS_Store"}:
            continue
        relative = path.relative_to(ENGINE_ROOT).as_posix().encode()
        data = path.read_bytes().replace(b"\r\n", b"\n")
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(len(data).to_bytes(8, "big"))
        digest.update(data)
    return digest.hexdigest()


def baseline_check() -> dict[str, Any]:
    baseline_path = ENGINE_ROOT / "fixtures/e2.1/baselines/e2-v1-freeze.json"
    baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    errors = []
    actual = {}
    for relative, expected in baseline["protected_files"].items():
        path = ENGINE_ROOT / relative
        value = digest_bytes(path.read_bytes()) if path.is_file() else None
        actual[relative] = value
        if value != expected:
            errors.append(f"protected file changed: {relative}")
    return synthetic_check("e21-v1-baseline", errors, actual)


def schema_check() -> dict[str, Any]:
    manifest = json.loads((ENGINE_ROOT / "schemas/e2.1-manifest.json").read_text(encoding="utf-8"))
    errors = []
    schemas = {}
    for name in manifest["schemas"]:
        path = ENGINE_ROOT / "schemas" / name
        try:
            parsed = json.loads(path.read_text(encoding="utf-8"))
            schemas[name] = parsed.get("$id") or parsed.get("schema")
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{name}: {error}")
    if manifest.get("status") != "FROZEN":
        errors.append("schema manifest is not FROZEN")
    return synthetic_check("e21-v2-schemas", errors, schemas)


def fixture_check() -> tuple[dict[str, Any], set[str], set[str]]:
    manifest_path = ENGINE_ROOT / "fixtures/e2.1/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    errors: list[str] = []
    ids: set[str] = set()
    fixture_paths: list[str] = []
    sources = "\n".join(
        path.read_text(encoding="utf-8")
        for path in (ENGINE_ROOT / "crates").rglob("*.rs")
    ) + Path(__file__).read_text(encoding="utf-8")
    for entry in manifest["fixtures"]:
        fixture_id = entry["id"]
        if fixture_id in ids:
            errors.append(f"duplicate fixture id: {fixture_id}")
        ids.add(fixture_id)
        path = ENGINE_ROOT / "fixtures/e2.1" / entry["path"]
        fixture_paths.append(entry["path"])
        if not path.is_file():
            errors.append(f"missing fixture: {entry['path']}")
            continue
        try:
            json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as error:
            errors.append(f"invalid fixture JSON {entry['path']}: {error}")
        if entry.get("test_binding") and path.name not in sources:
            errors.append(f"fixture is not loaded by a test or runner: {path.name}")

    pcal = json.loads(
        (ENGINE_ROOT / "fixtures/e2.1/statistical/PCAL-E2.1-v2.json").read_text(encoding="utf-8")
    )
    pval = json.loads(
        (ENGINE_ROOT / "fixtures/e2.1/statistical/PVAL-E2.1-v2.json").read_text(encoding="utf-8")
    )
    pcal_seeds, pval_seeds = set(pcal["seeds"]), set(pval["seeds"])
    if len(pcal_seeds) != 32 or len(pval_seeds) != 32:
        errors.append("PCAL and PVAL must each contain 32 unique seeds")
    if pcal_seeds & pval_seeds:
        errors.append("PCAL and PVAL seed sets overlap")
    if manifest.get("status") != "FROZEN":
        errors.append("fixture manifest is not FROZEN")
    evidence = {"fixture_ids": sorted(ids), "fixture_paths": sorted(fixture_paths)}
    return synthetic_check("e21-fixture-audit", errors, evidence), pcal_seeds, pval_seeds


def cli_workflow_check() -> dict[str, Any]:
    errors: list[str] = []
    evidence: dict[str, Any] = {}
    executable = ENGINE_ROOT / "target/release" / (
        "terrain-cli.exe" if platform.system() == "Windows" else "terrain-cli"
    )

    def invoke(arguments: list[str]) -> dict[str, Any] | None:
        completed = subprocess.run(
            [str(executable), *arguments],
            cwd=ENGINE_ROOT,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
        )
        if completed.returncode != 0:
            errors.append(f"{' '.join(arguments[:2])}: {completed.stderr.strip()}")
            return None
        try:
            return json.loads(completed.stdout.strip().splitlines()[-1])
        except (IndexError, json.JSONDecodeError) as error:
            errors.append(f"{' '.join(arguments[:2])}: invalid JSON: {error}")
            return None

    if not executable.is_file():
        return synthetic_check("e21-cli-workflow", [f"missing executable: {executable}"], evidence)

    with tempfile.TemporaryDirectory(prefix="e21-cli-") as directory:
        store = Path(directory) / "store"
        spatial = invoke([
            "sphere-v2", "build", "--store-root", str(store),
            "--release-id", "spatial-cli-v2",
        ])
        if not spatial:
            return synthetic_check("e21-cli-workflow", errors, evidence)
        spatial_dir = Path(spatial["path"])
        world = invoke([
            "world-spec-v2", "build", "--seed", "E21-CLI-01",
            "--spatial-release-dir", str(spatial_dir), "--store-root", str(store),
        ])
        if not world:
            return synthetic_check("e21-cli-workflow", errors, evidence)
        intent_id = world["world_intent"]["release_id"]
        generation_id = world["generation_spec"]["release_id"]
        intent_dir = store / "releases" / intent_id
        generation_dir = store / "releases" / generation_id
        tectonic = invoke([
            "tectonics-v2", "build",
            "--world-intent-file", str(intent_dir / "world-intent-v2.json"),
            "--generation-spec-file", str(generation_dir / "generation-spec-v2.json"),
            "--store-root", str(store), "--release-id", "tectonics-cli-v2",
        ])
        if not tectonic:
            return synthetic_check("e21-cli-workflow", errors, evidence)
        tectonic_dir = Path(tectonic["path"])
        m1 = invoke([
            "terrain-input-v2", "build", "--store-root", str(store),
            "--world-intent-release-dir", str(intent_dir),
            "--generation-spec-release-dir", str(generation_dir),
            "--spatial-release-dir", str(spatial_dir),
            "--tectonic-release-dir", str(tectonic_dir),
        ])
        if not m1:
            return synthetic_check("e21-cli-workflow", errors, evidence)
        m1_dir = Path(m1["path"])
        validations = {
            "spatial": invoke(["sphere-v2", "validate", str(spatial_dir)]),
            "world": invoke([
                "world-spec-v2", "validate",
                "--world-intent-file", str(intent_dir / "world-intent-v2.json"),
                "--generation-spec-file", str(generation_dir / "generation-spec-v2.json"),
            ]),
            "tectonic": invoke(["tectonics-v2", "validate", str(tectonic_dir)]),
            "m1": invoke([
                "terrain-input-v2", "validate", "--store-root", str(store),
                "--release-dir", str(m1_dir),
            ]),
        }
        for name, result in validations.items():
            if not result or result.get("status") != "VALID":
                errors.append(f"{name} v2 validation did not return VALID")
        if m1.get("admission", {}).get("conclusion") != "DIAGNOSTIC_ONLY":
            errors.append("natural-world M1 admission is not DIAGNOSTIC_ONLY")
        for name, result in (("spatial", spatial), ("tectonic", tectonic), ("m1", m1)):
            if result.get("durability") != "DURABILITY_NOT_PROVEN":
                errors.append(f"{name} overstated durability")
            if result.get("visibility") != "ATOMICALLY_VISIBLE":
                errors.append(f"{name} did not report atomic visibility")
        evidence = {
            "world_id": intent_id,
            "generation_identity": generation_id,
            "spatial_manifest_sha256": spatial["manifest_sha256"],
            "tectonic_manifest_sha256": tectonic["manifest_sha256"],
            "m1_release_sha256": m1["release_sha256"],
            "m1_admission": m1["admission"]["conclusion"],
            "validations": {name: value.get("status") if value else None for name, value in validations.items()},
        }
    return synthetic_check("e21-cli-workflow", errors, evidence)


def cargo_test(name: str, package: str, test_name: str) -> dict[str, Any]:
    return run_check(
        name,
        [
            "cargo", "test", "--quiet", "--locked", "--offline", "--release",
            "-p", package, test_name, "--", "--exact",
        ],
    )


def gate(gate_id: str, assertion: str, dependencies: list[str], checks: dict[str, dict[str, Any]]) -> dict[str, Any]:
    selected = [checks[name] for name in dependencies]
    passed = bool(selected) and all(item["status"] == "PASS" for item in selected)
    evidence = {
        "gate": gate_id,
        "assertion": assertion,
        "inputs": dependencies,
        "outputs": [item["output_sha256"] for item in selected],
    }
    return {
        "status": "PASS" if passed else "FAIL",
        "assertion": assertion,
        "checks": dependencies,
        "evidence_sha256": digest_bytes(canonical(evidence)),
    }


def pending_gate(assertion: str, reason: str) -> dict[str, Any]:
    evidence = {"assertion": assertion, "reason": reason, "status": "NOT_EVALUATED"}
    return {
        "status": "NOT_EVALUATED",
        "assertion": assertion,
        "reason": reason,
        "checks": [],
        "evidence_sha256": digest_bytes(canonical(evidence)),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    collected = [baseline_check(), schema_check()]
    fixture_audit, pcal_seeds, pval_seeds = fixture_check()
    collected.append(fixture_audit)
    collected.extend([
        cargo_test("identity-layering", "world-spec", "v2::tests::generation_identity_binds_exact_spatial_release_and_models"),
        cargo_test("fixed-world-isolation", "world-spec", "v2::tests::identity_fixture_is_executable"),
        cargo_test("skeleton-stability", "tectonics-core", "v2::tests::fixed_fixture_hash_enters_and_protects_the_skeleton"),
        cargo_test("activity-metamorphism", "tectonics-core", "v2::tests::activity_fixture_is_executable"),
        cargo_test("boundary-separation", "tectonics-core", "v2::tests::activity_preference_does_not_change_the_v2_skeleton"),
        cargo_test("m1-self-consistency", "terrain-input", "v2::tests::full_admission_recomputes_a_valid_diagnostic_release"),
        cargo_test("m1-full-admission", "terrain-input", "v2::tests::forged_claim_fixture_is_executable"),
        cargo_test("m1-filesystem-admission", "terrain-input", "v2::tests::filesystem_resolver_closes_full_m1_admission_from_published_bytes"),
        cargo_test("edge-geometry", "sphere-core", "v2::tests::reverse_edge_geometry_has_stable_identity_and_opposite_normal"),
        cargo_test("query-semantics", "sphere-core", "v2::tests::query_states_are_explicit"),
        cargo_test("patch-reconstruction", "sphere-core", "v2::tests::cross_face_patch_fixture_is_executable"),
        run_check("deterministic-numeric", ["cargo", "test", "--quiet", "--locked", "--offline", "--release", "-p", "deterministic-numeric"]),
        cargo_test("parallel-metamorphism", "tectonics-core", "v2::tests::fixed_shards_are_identical_across_workers_and_completion_orders"),
        run_check("store-semantics", ["cargo", "test", "--quiet", "--locked", "--offline", "--release", "-p", "release-store"]),
        run_check("pcal-execution", ["cargo", "run", "--quiet", "--locked", "--offline", "--release", "-p", "tectonics-core", "--example", "e21_seed_suite", "--", "fixtures/e2.1/statistical/PCAL-E2.1-v2.json"]),
        run_check("pval-execution", ["cargo", "run", "--quiet", "--locked", "--offline", "--release", "-p", "tectonics-core", "--example", "e21_seed_suite", "--", "fixtures/e2.1/statistical/PVAL-E2.1-v2.json"]),
        run_check("cargo-fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        run_check("cargo-clippy", ["cargo", "clippy", "--quiet", "--locked", "--offline", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"]),
        run_check("cargo-test-release", ["cargo", "test", "--quiet", "--locked", "--offline", "--release", "--workspace", "--all-targets"]),
        run_check(
            "python-oracle",
            [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-v"],
            ENGINE_ROOT / "reference/python",
        ),
        run_check("cargo-build-cli", ["cargo", "build", "--quiet", "--locked", "--offline", "--release", "-p", "terrain-cli"]),
    ])
    collected.append(cli_workflow_check())
    checks = {item["name"]: item for item in collected}

    pcal = last_json(checks["pcal-execution"])
    pval = last_json(checks["pval-execution"])
    if not pcal or pcal.get("world_count") != 32 or not pcal.get("pass"):
        checks["pcal-execution"]["status"] = "FAIL"
    if not pval or pval.get("world_count") != 32 or not pval.get("pass"):
        checks["pval-execution"]["status"] = "FAIL"
    if pcal_seeds & pval_seeds:
        checks["pcal-execution"]["status"] = "FAIL"
        checks["pval-execution"]["status"] = "FAIL"

    gates = {
        "E21-G01": gate("E21-G01", "v1基线未改变", ["e21-v1-baseline"], checks),
        "E21-G02": gate(
            "E21-G02",
            "v2模式、规范编码与CLI发布重开",
            ["e21-v2-schemas", "cargo-test-release", "cargo-build-cli", "e21-cli-workflow"],
            checks,
        ),
        "E21-G03": gate("E21-G03", "世界与生成身份分层", ["identity-layering"], checks),
        "E21-G04": gate("E21-G04", "固定测试身份隔离", ["fixed-world-isolation"], checks),
        "E21-G05": gate("E21-G05", "板块骨架稳定", ["skeleton-stability"], checks),
        "E21-G06": gate("E21-G06", "活动倍率单因素关系", ["activity-metamorphism"], checks),
        "E21-G07": gate("E21-G07", "边界类型与强度分离", ["boundary-separation"], checks),
        "E21-G08": gate("E21-G08", "M1结构与自身自洽", ["m1-self-consistency"], checks),
        "E21-G09": gate("E21-G09", "M1完整准入重建", ["m1-full-admission", "m1-filesystem-admission"], checks),
        "E21-G10": gate("E21-G10", "E1解析与边几何", ["edge-geometry", "query-semantics"], checks),
        "E21-G11": gate("E21-G11", "补丁谱系、晕区与2:1平衡", ["patch-reconstruction"], checks),
        "E21-G12": gate("E21-G12", "D0/D1数值确定性", ["deterministic-numeric", "python-oracle"], checks),
        "E21-G13": gate("E21-G13", "真实1/2/4线程确定性", ["parallel-metamorphism"], checks),
        "E21-G14": gate("E21-G14", "路径、事务和冲突处理", ["store-semantics"], checks),
        "E21-G15": gate("E21-G15", "PCAL实际执行", ["pcal-execution"], checks),
        "E21-G16": gate("E21-G16", "PVAL独立执行", ["pval-execution"], checks),
        "E21-G17": gate("E21-G17", "固定件直接加载与覆盖", ["e21-fixture-audit", "cargo-test-release"], checks),
        "E21-G18": pending_gate("平台同源和同检查集合", "等待Windows原生报告"),
        "E21-G19": pending_gate("跨平台权威证据一致", "等待macOS与Windows比较"),
        "E21-G20": pending_gate("冻结证据包完整", "等待E2.1-10冻结"),
    }
    stage_pass = all(gates[f"E21-G{number:02d}"]["status"] == "PASS" for number in range(1, 18))
    check_set = {
        gate_id: {"assertion": value["assertion"], "checks": value["checks"]}
        for gate_id, value in gates.items()
    }
    authoritative = {
        "stage": "E2.1-platform-candidate",
        "stage_pass": stage_pass,
        "pcal_evidence_sha256": pcal.get("evidence_sha256") if pcal else None,
        "pval_evidence_sha256": pval.get("evidence_sha256") if pval else None,
        "pcal_worlds": pcal.get("worlds") if pcal else None,
        "pval_worlds": pval.get("worlds") if pval else None,
        "durability": "DURABILITY_NOT_PROVEN",
        "cli_workflow_sha256": checks["e21-cli-workflow"]["output_sha256"],
    }
    report = {
        "report_schema": "E2.1-PLATFORM-REPORT-v1",
        "status": "NOT_EVALUATED" if stage_pass else "FAIL",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
        "source": {
            "source_tree_sha256": source_tree_sha256(),
            "cargo_lock_sha256": digest_bytes((ENGINE_ROOT / "Cargo.lock").read_bytes()),
            "fixture_manifest_sha256": digest_bytes((ENGINE_ROOT / "fixtures/e2.1/manifest.json").read_bytes()),
            "schema_manifest_sha256": digest_bytes((ENGINE_ROOT / "schemas/e2.1-manifest.json").read_bytes()),
        },
        "check_set_sha256": digest_bytes(canonical(check_set)),
        "gates": gates,
        "checks": collected,
        "authoritative_evidence": authoritative,
        "authoritative_evidence_sha256": digest_bytes(canonical(authoritative)),
    }
    platform_name = f"platform-{platform.system().lower()}-{platform.machine().lower()}.json"
    output = args.output or ENGINE_ROOT / "reports/e2.1" / platform_name
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"E2.1 stage report: {output}")
    print(f"E2.1 local candidate checks: {'PASS' if stage_pass else 'FAIL'}")
    return 0 if stage_pass else 1


if __name__ == "__main__":
    raise SystemExit(main())
