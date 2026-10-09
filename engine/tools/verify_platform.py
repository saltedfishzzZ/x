#!/usr/bin/env python3
"""运行 T0/E0 平台验证并生成结构化报告。"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import re
import subprocess
import sys
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
        "output_sha256": hashlib.sha256(output.encode("utf-8")).hexdigest(),
        "output": output,
    }


def command_version(command: list[str]) -> str:
    completed = subprocess.run(
        command,
        cwd=ENGINE_ROOT,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
    )
    return (completed.stdout or completed.stderr).strip()


def parse_json_output(check: dict[str, Any]) -> dict[str, Any] | None:
    lines = [line for line in check["output"].splitlines() if line.startswith("{")]
    if not lines:
        return None
    return json.loads(lines[-1])


def validate_json_files() -> tuple[int, list[str]]:
    files = sorted(
        path
        for path in ENGINE_ROOT.rglob("*.json")
        if "target" not in path.parts
    )
    errors: list[str] = []
    for path in files:
        try:
            json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            errors.append(f"{path.relative_to(ENGINE_ROOT)}: {error}")
    return len(files), errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        help="报告路径；默认按操作系统和架构写入 technical-spikes 目录",
    )
    arguments = parser.parse_args()

    python_supported = sys.version_info[:2] >= MINIMUM_PYTHON
    python_version_output = platform.python_version()
    python_version_status = "PASS" if python_supported else "FAIL"
    print(f"[{python_version_status}] python-version ({python_version_output})")
    checks = [
        {
            "name": "python-version",
            "status": python_version_status,
            "return_code": 0 if python_supported else 1,
            "duration_seconds": 0.0,
            "output_sha256": hashlib.sha256(
                python_version_output.encode("utf-8")
            ).hexdigest(),
            "output": python_version_output,
        },
        run_check("cargo-fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        run_check(
            "cargo-clippy",
            [
                "cargo",
                "clippy",
                "--locked",
                "--offline",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
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
            "cli-inspect",
            [
                "cargo",
                "run",
                "--quiet",
                "--locked",
                "--offline",
                "--release",
                "-p",
                "terrain-cli",
                "--",
                "inspect",
            ],
        ),
        run_check(
            "cli-diagnostics",
            [
                "cargo",
                "run",
                "--quiet",
                "--locked",
                "--offline",
                "--release",
                "-p",
                "terrain-cli",
                "--",
                "diagnostics",
            ],
        ),
        run_check(
            "cli-conformance",
            [
                "cargo",
                "run",
                "--quiet",
                "--locked",
                "--offline",
                "--release",
                "-p",
                "terrain-cli",
                "--",
                "conformance",
            ],
        ),
    ]

    json_count, json_errors = validate_json_files()
    json_status = "PASS" if not json_errors else "FAIL"
    print(f"[{json_status}] json-fixtures ({json_count})")
    checks.append(
        {
            "name": "json-fixtures",
            "status": json_status,
            "return_code": 0 if not json_errors else 1,
            "duration_seconds": 0.0,
            "output_sha256": hashlib.sha256(
                "\n".join(json_errors).encode("utf-8")
            ).hexdigest(),
            "output": "\n".join(json_errors),
        }
    )

    test_output = next(
        check["output"] for check in checks if check["name"] == "cargo-test-release"
    )
    rust_tests = sum(
        int(count)
        for count in re.findall(r"test result: ok\. (\d+) passed", test_output)
    )
    oracle_output = next(
        check["output"] for check in checks if check["name"] == "python-oracle"
    )
    oracle_match = re.search(r"Ran (\d+) tests?", oracle_output)
    oracle_tests = int(oracle_match.group(1)) if oracle_match else None

    inspect = parse_json_output(next(c for c in checks if c["name"] == "cli-inspect"))
    diagnostics = parse_json_output(
        next(c for c in checks if c["name"] == "cli-diagnostics")
    )
    conformance = parse_json_output(
        next(c for c in checks if c["name"] == "cli-conformance")
    )
    overall = "PLATFORM_PASS" if all(c["status"] == "PASS" for c in checks) else "FAIL"

    report = {
        "report_schema": "T0-E0-PLATFORM-REPORT-v1",
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
            "rust_tests_passed": rust_tests,
            "python_oracle_tests_passed": oracle_tests,
            "json_files_validated": json_count,
        },
        "evidence": {
            "inspect": inspect,
            "diagnostics": diagnostics,
            "conformance": conformance,
        },
        "checks": [
            {key: value for key, value in check.items() if key != "output"}
            for check in checks
        ],
    }

    system_slug = re.sub(r"[^a-z0-9]+", "-", platform.system().lower()).strip("-")
    machine_slug = re.sub(r"[^a-z0-9]+", "-", platform.machine().lower()).strip("-")
    output = arguments.output or (
        ENGINE_ROOT
        / "reports"
        / "technical-spikes"
        / f"platform-{system_slug}-{machine_slug}.json"
    )
    if not output.is_absolute():
        output = ENGINE_ROOT / output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(f"报告：{output}")
    return 0 if overall == "PLATFORM_PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
