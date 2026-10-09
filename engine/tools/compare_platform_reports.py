#!/usr/bin/env python3
"""比较两个 T0/E0 原生平台报告。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


MINIMUM_PYTHON = (3, 11)


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def nested(report: dict[str, Any], path: str) -> Any:
    value: Any = report
    for component in path.split("."):
        value = value[component]
    return value


def python_version(report: dict[str, Any]) -> tuple[int, int]:
    components = report["toolchain"]["python"].split(".")
    return int(components[0]), int(components[1])


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path)
    parser.add_argument("--area-tolerance", type=float, default=1.0e-15)
    arguments = parser.parse_args()
    left = read(arguments.left)
    right = read(arguments.right)

    differences: list[dict[str, Any]] = []
    exact_paths = [
        "report_schema",
        "status",
        "toolchain.rustc",
        "toolchain.cargo",
        "evidence.inspect",
        "evidence.diagnostics.cube_rotations",
        "evidence.conformance.canonical_json_version",
        "evidence.conformance.canonical_json",
        "evidence.conformance.canonical_json_sha256",
    ]
    for path in exact_paths:
        left_value = nested(left, path)
        right_value = nested(right, path)
        if left_value != right_value:
            differences.append(
                {"path": path, "left": left_value, "right": right_value}
            )

    for label, report in (("left", left), ("right", right)):
        version = python_version(report)
        if version < MINIMUM_PYTHON:
            differences.append(
                {
                    "path": f"{label}.toolchain.python",
                    "value": report["toolchain"]["python"],
                    "minimum": ".".join(map(str, MINIMUM_PYTHON)),
                }
            )

    area_path = "evidence.diagnostics.l8_relative_area_error"
    left_area = float(nested(left, area_path))
    right_area = float(nested(right, area_path))
    if abs(left_area - right_area) > arguments.area_tolerance:
        differences.append(
            {
                "path": area_path,
                "left": left_area,
                "right": right_area,
                "absolute_difference": abs(left_area - right_area),
                "tolerance": arguments.area_tolerance,
            }
        )

    result = {
        "comparison_schema": "T0-E0-PLATFORM-COMPARISON-v1",
        "status": "PASS" if not differences else "FAIL",
        "left_platform": left["platform"],
        "right_platform": right["platform"],
        "differences": differences,
    }
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if not differences else 1


if __name__ == "__main__":
    raise SystemExit(main())
