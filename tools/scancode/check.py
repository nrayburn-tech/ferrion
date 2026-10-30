"""Checks a ScanCode JSON report for licenses and copyright holders that don't belong in Ferrion.

Usage: python3 tools/scancode/check.py <scancode.json>

Fails if any file outside the exempt list has a license detection other than MIT or Apache-2.0, or a
copyright holder other than Ferrion's. Either usually means code was copied from somewhere else.
See the clean-room policy in CONTRIBUTING.md.
"""

import json
import re
import sys

ALLOWED_LICENSES = {"mit", "apache-2.0"}
ALLOWED_HOLDERS = {"Nicholas Rayburn"}

# Files that name or contain other licenses on purpose.
EXEMPT_FILES = {"LICENSE-MIT", "LICENSE-APACHE", "CONTRIBUTING.md", "deny.toml"}
EXEMPT_PREFIXES = ("docs/", "tools/scancode/")


def license_keys(expression):
    words = re.split(r"[\s()]+", expression.lower())
    return {word for word in words if word and word not in {"and", "or", "with"}}


def main(report_path):
    with open(report_path, encoding="utf-8") as report:
        report = json.load(report)
    problems = []
    for file in report["files"]:
        path = file["path"]
        if file["type"] != "file" or path in EXEMPT_FILES or path.startswith(EXEMPT_PREFIXES):
            continue
        expression = file.get("detected_license_expression") or ""
        unexpected = license_keys(expression) - ALLOWED_LICENSES
        if unexpected:
            problems.append(f"{path}: license {', '.join(sorted(unexpected))}")
        for holder in file.get("holders", []):
            if holder["holder"] not in ALLOWED_HOLDERS:
                problems.append(f"{path}:{holder['start_line']}: copyright holder {holder['holder']}")
    for problem in problems:
        print(problem)
    if problems:
        print(f"\n{len(problems)} problem(s). See the clean-room policy in CONTRIBUTING.md.")
        return 1
    print("No unexpected licenses or copyright holders.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
