#!/usr/bin/env python3
"""比较macOS与Windows E2原生报告。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def compare(left: dict[str, Any], right: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if left.get("report_schema") != "E2-PLATFORM-REPORT-v1" or right.get("report_schema") != "E2-PLATFORM-REPORT-v1":
        errors.append("report schema mismatch")
    if left.get("status") != "PLATFORM_PASS" or right.get("status") != "PLATFORM_PASS":
        errors.append("both reports must be PLATFORM_PASS")
    systems = {left.get("platform", {}).get("system"), right.get("platform", {}).get("system")}
    if systems != {"Darwin", "Windows"}:
        errors.append(f"expected Darwin and Windows reports, got {sorted(str(v) for v in systems)}")
    if left.get("source_tree_sha256") != right.get("source_tree_sha256"):
        errors.append("source tree digests differ")
    if left.get("authoritative_evidence") != right.get("authoritative_evidence"):
        errors.append("authoritative E2 evidence differs")
    if left.get("authoritative_evidence_sha256") != right.get("authoritative_evidence_sha256"):
        errors.append("authoritative evidence digests differ")
    for report, label in ((left, "left"), (right, "right")):
        if report.get("gates", {}).get("E2-G17") != "PLATFORM_EVIDENCE_READY":
            errors.append(f"{label} report is not ready for E2-G17 comparison")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    left = json.loads(args.left.read_text(encoding="utf-8"))
    right = json.loads(args.right.read_text(encoding="utf-8"))
    errors = compare(left, right)
    result = {
        "report_schema": "E2-CROSS-PLATFORM-COMPARISON-v1",
        "status": "CROSS_PLATFORM_PASS" if not errors else "FAIL",
        "left": str(args.left),
        "right": str(args.right),
        "authoritative_evidence_sha256": left.get("authoritative_evidence_sha256") if not errors else None,
        "errors": errors,
    }
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False))
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
