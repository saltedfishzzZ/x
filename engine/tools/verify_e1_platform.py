#!/usr/bin/env python3
"""运行 E1 球面空间层平台验证并生成结构化报告。"""

from __future__ import annotations

import argparse
import hashlib
import json
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


def run_check(name: str, command: list[str], cwd: Path = ENGINE_ROOT) -> dict[str, Any]:
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    version_ok = sys.version_info[:2] >= MINIMUM_PYTHON
    checks = [
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
                "--all-targets", "--", "-D", "warnings",
            ],
        ),
        run_check(
            "cargo-test-release",
            ["cargo", "test", "--locked", "--offline", "--release", "--workspace"],
        ),
        run_check(
            "python-oracle",
            [sys.executable, "-m", "unittest", "discover", "-s", "tests", "-v"],
            ENGINE_ROOT / "reference" / "python",
        ),
        run_check(
            "sphere-inspect",
            [
                "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                "-p", "terrain-cli", "--", "sphere", "inspect",
            ],
        ),
        run_check(
            "sphere-benchmark",
            [
                "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                "-p", "sphere-core", "--example", "e1_benchmark",
            ],
        ),
    ]

    with tempfile.TemporaryDirectory(prefix="terrain-e1-") as directory:
        store = Path(directory) / "store"
        build = run_check(
            "sphere-build",
            [
                "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                "-p", "terrain-cli", "--", "sphere", "build",
                "--store-root", str(store), "--release-id", "e1-conformance",
            ],
        )
        checks.append(build)
        release_dir = store / "releases" / "e1-conformance"
        validate = run_check(
            "sphere-validate",
            [
                "cargo", "run", "--quiet", "--locked", "--offline", "--release",
                "-p", "terrain-cli", "--", "sphere", "validate", str(release_dir),
            ],
        )
        checks.append(validate)
        package_bytes = (
            sum(path.stat().st_size for path in release_dir.rglob("*") if path.is_file())
            if release_dir.exists()
            else None
        )

    fixture_files = sorted(
        path for path in (ENGINE_ROOT / "fixtures").rglob("*.json")
    )
    fixture_errors: list[str] = []
    for path in fixture_files:
        try:
            json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            fixture_errors.append(f"{path.relative_to(ENGINE_ROOT)}: {error}")
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

    test_output = next(item["output"] for item in checks if item["name"] == "cargo-test-release")
    rust_tests = sum(int(value) for value in re.findall(r"test result: ok\. (\d+) passed", test_output))
    oracle_output = next(item["output"] for item in checks if item["name"] == "python-oracle")
    oracle_match = re.search(r"Ran (\d+) tests?", oracle_output)
    inspect = last_json(next(item for item in checks if item["name"] == "sphere-inspect"))
    benchmark = last_json(next(item for item in checks if item["name"] == "sphere-benchmark"))
    published = last_json(next(item for item in checks if item["name"] == "sphere-build"))
    deterministic = {
        "identity_format_version": inspect.get("identity_format_version") if inspect else None,
        "l0_topology": inspect.get("validation_evidence", {}).get("l0_topology") if inspect else None,
        "l8_relative_area_tolerance": inspect.get("validation_evidence", {}).get("l8_relative_area_tolerance") if inspect else None,
        "manifest_sha256": published.get("manifest_sha256") if published else None,
    }
    deterministic_bytes = json.dumps(
        deterministic, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode()
    performance_pass = bool(
        benchmark
        and benchmark["geometry_l8"]["pass"]
        and benchmark["locate_l10"]["pass"]
        and benchmark["patch_e1f06"]["pass"]
        and package_bytes is not None
        and package_bytes < 100 * 1024 * 1024
    )
    overall = (
        "PLATFORM_PASS"
        if all(item["status"] == "PASS" for item in checks) and performance_pass
        else "FAIL"
    )
    report = {
        "report_schema": "T0-E1-PLATFORM-REPORT-v1",
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
        "counts": {
            "rust_tests": rust_tests,
            "python_oracle_tests": int(oracle_match.group(1)) if oracle_match else None,
            "json_fixtures": len(fixture_files),
        },
        "deterministic_evidence": deterministic,
        "deterministic_evidence_sha256": hashlib.sha256(deterministic_bytes).hexdigest(),
        "performance": benchmark,
        "release_package_bytes": package_bytes,
        "checks": checks,
    }
    output = args.output or (
        ENGINE_ROOT
        / "reports"
        / "e1"
        / f"platform-{platform.system().lower()}-{platform.machine().lower()}.json"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"[{overall}] report: {output}")
    return 0 if overall == "PLATFORM_PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
