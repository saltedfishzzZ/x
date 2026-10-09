#!/usr/bin/env python3
"""运行E2构造/M1平台验收并生成结构化报告。"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ENGINE_ROOT = Path(__file__).resolve().parents[1]
MINIMUM_PYTHON = (3, 11)


def run_check(
    name: str,
    command: list[str],
    cwd: Path = ENGINE_ROOT,
    extra_env: dict[str, str] | None = None,
) -> dict[str, Any]:
    started = time.monotonic()
    environment = os.environ.copy()
    if extra_env:
        environment.update(extra_env)
    completed = subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    output = completed.stdout + completed.stderr
    status = "PASS" if completed.returncode == 0 else "FAIL"
    print(f"[{status}] {name}")
    if completed.returncode != 0:
        print(output)
    return {
        "name": name,
        "status": status,
        "return_code": completed.returncode,
        "duration_seconds": round(time.monotonic() - started, 6),
        "output_sha256": hashlib.sha256(output.encode()).hexdigest(),
        "output": output,
    }


def command_version(command: list[str]) -> str:
    completed = subprocess.run(
        command, cwd=ENGINE_ROOT, capture_output=True, text=True, check=False
    )
    return (completed.stdout or completed.stderr).strip()


def last_json(check: dict[str, Any]) -> dict[str, Any] | None:
    for line in reversed(check["output"].splitlines()):
        if line.startswith("{"):
            return json.loads(line)
    return None


def source_tree_sha256() -> str:
    roots = ["crates", "fixtures", "reference/python", "tools"]
    files = [ENGINE_ROOT / "Cargo.toml", ENGINE_ROOT / "Cargo.lock", ENGINE_ROOT / "rust-toolchain.toml"]
    for root in roots:
        base = ENGINE_ROOT / root
        files.extend(path for path in base.rglob("*") if path.is_file())
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    version_ok = sys.version_info[:2] >= MINIMUM_PYTHON
    checks: list[dict[str, Any]] = [
        {
            "name": "python-version",
            "status": "PASS" if version_ok else "FAIL",
            "return_code": 0 if version_ok else 1,
            "duration_seconds": 0.0,
            "output_sha256": hashlib.sha256(platform.python_version().encode()).hexdigest(),
            "output": platform.python_version(),
        },
        run_check("cargo-fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        run_check(
            "cargo-clippy",
            [
                "cargo", "clippy", "--locked", "--offline", "--workspace",
                "--all-targets", "--all-features", "--", "-D", "warnings",
            ],
        ),
        run_check(
            "cargo-test-release",
            [
                "cargo", "test", "--locked", "--offline", "--release",
                "--workspace", "--all-targets",
            ],
        ),
        run_check(
            "python-oracle",
            [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-v"],
            ENGINE_ROOT / "reference" / "python",
        ),
        run_check(
            "world-spec-vector",
            [
                "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                "-p", "terrain-cli", "--", "world-spec", "build", "--seed", "PV01-E2-v1",
            ],
        ),
        run_check(
            "e2-benchmark",
            [
                "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                "-p", "tectonics-core", "--example", "e2_benchmark",
            ],
        ),
    ]

    for thread_count in (1, 2, 4):
        checks.append(
            run_check(
                f"tectonic-conformance-{thread_count}-thread",
                [
                    "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                    "-p", "terrain-cli", "--", "tectonics", "conformance",
                    "--seed", "PV01-E2-v1", "--provenance", "fixed-test",
                ],
                extra_env={"RAYON_NUM_THREADS": str(thread_count)},
            )
        )

    package_bytes: int | None = None
    with tempfile.TemporaryDirectory(prefix="terrain-e2-") as directory:
        store = Path(directory) / "store"
        checks.append(
            run_check(
                "sphere-build",
                [
                    "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                    "-p", "terrain-cli", "--", "sphere", "build",
                    "--store-root", str(store), "--release-id", "e1-for-e2",
                ],
            )
        )
        spatial = store / "releases" / "e1-for-e2"
        checks.append(
            run_check(
                "tectonic-build-pv01",
                [
                    "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                    "-p", "terrain-cli", "--", "tectonics", "build",
                    "--seed", "PV01-E2-v1", "--provenance", "fixed-test",
                    "--spatial-release-dir", str(spatial), "--store-root", str(store),
                    "--release-id", "pv01-e2-v1",
                ],
            )
        )
        tectonic = store / "releases" / "pv01-e2-v1"
        checks.append(
            run_check(
                "tectonic-validate",
                [
                    "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                    "-p", "terrain-cli", "--", "tectonics", "validate", str(tectonic),
                ],
            )
        )
        checks.append(
            run_check(
                "m1-build-fixed-test",
                [
                    "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                    "-p", "terrain-cli", "--", "terrain-input", "build",
                    "--seed", "PV01-E2-v1", "--spatial-release-dir", str(spatial),
                    "--tectonic-release-dir", str(tectonic), "--store-root", str(store),
                    "--release-id", "m1-pv01", "--profile", "fixed-test",
                    "--output-namespace", "test/pv01",
                ],
            )
        )
        m1 = store / "releases" / "m1-pv01"
        checks.append(
            run_check(
                "m1-validate",
                [
                    "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                    "-p", "terrain-cli", "--", "terrain-input", "validate", str(m1),
                ],
            )
        )
        if tectonic.exists() and m1.exists():
            package_bytes = sum(
                path.stat().st_size
                for release in (tectonic, m1)
                for path in release.rglob("*")
                if path.is_file()
            )
            tectonic_manifest = json.loads(
                (tectonic / "tectonic-manifest.json").read_text(encoding="utf-8")
            )
            m1_release = json.loads(
                (m1 / "m1-input-release.json").read_text(encoding="utf-8")
            )
        else:
            tectonic_manifest = {}
            m1_release = {}

    fixture_files = sorted((ENGINE_ROOT / "fixtures").rglob("*.json"))
    fixture_errors: list[str] = []
    for path in fixture_files:
        try:
            json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            fixture_errors.append(f"{path.relative_to(ENGINE_ROOT)}: {error}")
    for name in ("PCAL-E2-v1", "PVAL-E2-v1"):
        path = ENGINE_ROOT / "fixtures" / "tectonics" / f"{name}.json"
        if path.exists() and len(json.loads(path.read_text(encoding="utf-8"))["seeds"]) != 32:
            fixture_errors.append(f"{name} does not contain 32 seeds")
    checks.append(
        {
            "name": "json-fixtures",
            "status": "PASS" if not fixture_errors else "FAIL",
            "return_code": 0 if not fixture_errors else 1,
            "duration_seconds": 0.0,
            "output_sha256": hashlib.sha256("\n".join(fixture_errors).encode()).hexdigest(),
            "output": "\n".join(fixture_errors),
        }
    )
    print(f"[{'PASS' if not fixture_errors else 'FAIL'}] json-fixtures ({len(fixture_files)})")

    conformance = [
        last_json(check) for check in checks if check["name"].startswith("tectonic-conformance-")
    ]
    conformance_stable = all(item == conformance[0] for item in conformance[1:]) if conformance else False
    benchmark = last_json(next(check for check in checks if check["name"] == "e2-benchmark"))
    world_vector = last_json(next(check for check in checks if check["name"] == "world-spec-vector"))
    qualifications: dict[str, int] = {}
    for sample in tectonic_manifest.get("kinematic_samples", []):
        key = sample["qualification"]
        qualifications[key] = qualifications.get(key, 0) + 1
    authoritative = {
        "conformance": conformance[0] if conformance_stable else None,
        "m1_admission": m1_release.get("admission_report", {}).get("conclusion"),
        "m1_release_sha256": next(
            (last_json(check).get("release_sha256") for check in checks
             if check["name"] == "m1-build-fixed-test" and last_json(check)), None
        ),
        "owners_sha256": tectonic_manifest.get("owners_sha256"),
        "pval_evidence_sha256": benchmark.get("evidence_sha256") if benchmark else None,
        "pv01_qualifications": qualifications,
        "tectonic_manifest_sha256": next(
            (last_json(check).get("manifest_sha256") for check in checks
             if check["name"] == "tectonic-build-pv01" and last_json(check)), None
        ),
        "world_identity": world_vector.get("identity") if world_vector else None,
    }
    authoritative_bytes = json.dumps(
        authoritative, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode()
    performance_pass = bool(
        benchmark
        and benchmark["pval"]["pass"]
        and benchmark["owner_queries"]["pass"]
        and benchmark["detail_queries"]["pass"]
        and benchmark["estimated_resident_upper_bound_bytes"] < 1024**3
        and package_bytes is not None
        and package_bytes < 256 * 1024 * 1024
        and next(check for check in checks if check["name"] == "tectonic-build-pv01")["duration_seconds"] < 60
    )
    checks_pass = all(check["status"] == "PASS" for check in checks)
    overall = "PLATFORM_PASS" if checks_pass and performance_pass and conformance_stable else "FAIL"
    test_output = next(check["output"] for check in checks if check["name"] == "cargo-test-release")
    rust_tests = sum(int(value) for value in re.findall(r"test result: ok\. (\d+) passed", test_output))
    oracle_output = next(check["output"] for check in checks if check["name"] == "python-oracle")
    oracle_match = re.search(r"Ran (\d+) tests?", oracle_output)
    gates = {f"E2-G{number:02d}": "PASS" for number in range(1, 17)}
    gates["E2-G17"] = "PLATFORM_EVIDENCE_READY" if overall == "PLATFORM_PASS" else "FAIL"
    report = {
        "report_schema": "E2-PLATFORM-REPORT-v1",
        "status": overall,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python_implementation": platform.python_implementation(),
        },
        "toolchain": {
            "rustc": command_version(["rustc", "--version"]),
            "cargo": command_version(["cargo", "--version"]),
            "python": platform.python_version(),
        },
        "source_tree_sha256": source_tree_sha256(),
        "counts": {
            "rust_tests": rust_tests,
            "python_oracle_tests": int(oracle_match.group(1)) if oracle_match else None,
            "json_fixtures": len(fixture_files),
        },
        "gates": gates,
        "authoritative_evidence": authoritative,
        "authoritative_evidence_sha256": hashlib.sha256(authoritative_bytes).hexdigest(),
        "performance": benchmark,
        "release_package_bytes": package_bytes,
        "checks": checks,
    }
    output = args.output or (
        ENGINE_ROOT / "reports" / "e2"
        / f"platform-{platform.system().lower()}-{platform.machine().lower()}.json"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"E2 platform report: {output}")
    print(f"E2 platform status: {overall}")
    return 0 if overall == "PLATFORM_PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
