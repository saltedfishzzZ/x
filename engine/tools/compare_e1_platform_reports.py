#!/usr/bin/env python3
"""比较 macOS 与 Windows 的 E1 离散空间证据。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("first", type=Path)
    parser.add_argument("second", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    first = json.loads(args.first.read_text(encoding="utf-8"))
    second = json.loads(args.second.read_text(encoding="utf-8"))
    comparisons = {
        "both_platform_pass": first.get("status") == second.get("status") == "PLATFORM_PASS",
        "report_schema": first.get("report_schema") == second.get("report_schema") == "T0-E1-PLATFORM-REPORT-v1",
        "deterministic_evidence": first.get("deterministic_evidence") == second.get("deterministic_evidence"),
        "deterministic_evidence_sha256": first.get("deterministic_evidence_sha256") == second.get("deterministic_evidence_sha256"),
        "test_counts": first.get("counts") == second.get("counts"),
    }
    status = "CROSS_PLATFORM_PASS" if all(comparisons.values()) else "FAIL"
    result = {
        "report_schema": "T0-E1-CROSS-PLATFORM-COMPARISON-v1",
        "status": status,
        "first": str(args.first),
        "second": str(args.second),
        "comparisons": comparisons,
        "toolchain_versions_are_informational": True,
    }
    output = args.output or args.first.parent / "platform-comparison-macos-windows.json"
    output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False))
    return 0 if status == "CROSS_PLATFORM_PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
