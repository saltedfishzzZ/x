#!/usr/bin/env python3
"""严格比较E2.1 macOS与Windows候选报告。"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def canonical(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()


def load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("report_schema") != "E2.1-PLATFORM-REPORT-v1":
        raise ValueError(f"unsupported report schema: {path}")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("macos", type=Path)
    parser.add_argument("windows", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    macos = load(args.macos)
    windows = load(args.windows)
    comparisons: list[dict[str, Any]] = []

    def compare(name: str, left: Any, right: Any) -> None:
        comparisons.append({
            "name": name,
            "status": "PASS" if left == right else "FAIL",
            "macos": left,
            "windows": right,
        })

    for field in (
        "source_tree_sha256",
        "cargo_lock_sha256",
        "fixture_manifest_sha256",
        "schema_manifest_sha256",
    ):
        compare(field, macos["source"][field], windows["source"][field])
    compare("check_set_sha256", macos["check_set_sha256"], windows["check_set_sha256"])
    compare(
        "completed_gate_statuses",
        {key: value["status"] for key, value in macos["gates"].items() if key <= "E21-G17"},
        {key: value["status"] for key, value in windows["gates"].items() if key <= "E21-G17"},
    )
    compare(
        "check_names",
        [item["name"] for item in macos["checks"]],
        [item["name"] for item in windows["checks"]],
    )
    compare(
        "check_statuses",
        {item["name"]: item["status"] for item in macos["checks"]},
        {item["name"]: item["status"] for item in windows["checks"]},
    )
    compare(
        "authoritative_evidence_sha256",
        macos["authoritative_evidence_sha256"],
        windows["authoritative_evidence_sha256"],
    )

    passed = all(item["status"] == "PASS" for item in comparisons)
    evidence = {
        "source_tree_sha256": macos["source"]["source_tree_sha256"] if passed else None,
        "check_set_sha256": macos["check_set_sha256"] if passed else None,
        "authoritative_evidence_sha256": macos["authoritative_evidence_sha256"] if passed else None,
    }
    result = {
        "schema": "e2.1-platform-comparison-v1",
        "status": "CROSS_PLATFORM_PASS" if passed else "FAIL",
        "macos_report": args.macos.as_posix(),
        "windows_report": args.windows.as_posix(),
        "comparisons": comparisons,
        "gates": {
            "E21-G18": "PASS" if passed else "FAIL",
            "E21-G19": "PASS" if passed else "FAIL",
            "E21-G20": "NOT_EVALUATED",
        },
        "authoritative_evidence": evidence,
    }
    result["comparison_sha256"] = hashlib.sha256(canonical(result)).hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"E2.1 cross-platform report: {args.output}")
    print(f"E2.1 cross-platform status: {result['status']}")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
