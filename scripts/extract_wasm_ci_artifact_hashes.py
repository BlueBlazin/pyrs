#!/usr/bin/env python3
"""Extract uploaded artifact IDs + SHA256 digests from a GitHub Actions run log.

Supports parsing either:
1. live logs fetched via `gh run view <run-id> --log`, or
2. an existing local log file.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class ArtifactRecord:
    job: str
    step: str
    name: str
    artifact_id: str
    sha256: str

    def to_dict(self) -> dict[str, str]:
        return {
            "job": self.job,
            "step": self.step,
            "name": self.name,
            "artifact_id": self.artifact_id,
            "sha256": self.sha256,
        }


DIGEST_RE = re.compile(
    r"^(?P<job>[^\t]+)\t(?P<step>[^\t]+)\t[^\t]*SHA256 digest of uploaded artifact zip is (?P<sha>[0-9a-f]{64})$"
)

FINALIZED_RE = re.compile(
    r"^(?P<job>[^\t]+)\t(?P<step>[^\t]+)\t[^\t]*Artifact (?P<name>[^ ]+)\.zip successfully finalized\. Artifact ID (?P<artifact_id>\d+)$"
)


def run_gh(args: list[str]) -> str:
    proc = subprocess.run(
        args,
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or "unknown gh error")
    return proc.stdout


def parse_records(log_text: str) -> list[ArtifactRecord]:
    digest_by_step: dict[tuple[str, str], str] = {}
    records: list[ArtifactRecord] = []

    for raw_line in log_text.splitlines():
        digest_match = DIGEST_RE.match(raw_line)
        if digest_match:
            key = (digest_match.group("job"), digest_match.group("step"))
            digest_by_step[key] = digest_match.group("sha")
            continue

        finalized_match = FINALIZED_RE.match(raw_line)
        if finalized_match:
            job = finalized_match.group("job")
            step = finalized_match.group("step")
            key = (job, step)
            sha = digest_by_step.get(key, "")
            records.append(
                ArtifactRecord(
                    job=job,
                    step=step,
                    name=finalized_match.group("name"),
                    artifact_id=finalized_match.group("artifact_id"),
                    sha256=sha,
                )
            )

    records.sort(key=lambda record: record.name)
    return records


def format_markdown(payload: dict[str, Any]) -> str:
    lines = []
    run_id = payload.get("run_id")
    run_url = payload.get("run_url")
    head_sha = payload.get("head_sha")
    lines.append(f"- workflow run: [{run_id}]({run_url})")
    lines.append(f"- head commit: `{head_sha}`")
    lines.append("- artifact hashes:")
    for artifact in payload.get("artifacts", []):
        lines.append(f"  - `{artifact['name']}`")
        lines.append(f"    - artifact id: `{artifact['artifact_id']}`")
        lines.append(f"    - sha256: `{artifact['sha256']}`")
    return "\n".join(lines)


def build_payload(
    run_id: str | None,
    run_url: str | None,
    head_sha: str | None,
    records: list[ArtifactRecord],
) -> dict[str, Any]:
    return {
        "run_id": run_id,
        "run_url": run_url,
        "head_sha": head_sha,
        "artifact_count": len(records),
        "artifacts": [record.to_dict() for record in records],
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Extract uploaded artifact IDs and SHA256 digests from a GitHub Actions run log."
    )
    parser.add_argument("--run-id", help="GitHub Actions run id to query via gh CLI.")
    parser.add_argument("--log-file", help="Local log file to parse instead of gh run logs.")
    parser.add_argument("--run-url", help="Run URL metadata when using --log-file.")
    parser.add_argument("--head-sha", help="Head SHA metadata when using --log-file.")
    parser.add_argument(
        "--format",
        choices=("json", "markdown"),
        default="json",
        help="Output format (default: json).",
    )
    parser.add_argument("--out", help="Write output to file path (default: stdout).")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not args.run_id and not args.log_file:
        print("error: provide --run-id or --log-file", file=sys.stderr)
        return 1

    run_url: str | None = args.run_url
    head_sha: str | None = args.head_sha

    if args.log_file:
        log_text = Path(args.log_file).read_text(encoding="utf-8")
    else:
        assert args.run_id is not None
        metadata_text = run_gh(
            [
                "gh",
                "run",
                "view",
                args.run_id,
                "--json",
                "url,headSha",
            ]
        )
        metadata = json.loads(metadata_text)
        run_url = metadata.get("url")
        head_sha = metadata.get("headSha")
        log_text = run_gh(["gh", "run", "view", args.run_id, "--log"])

    records = parse_records(log_text)
    if not records:
        print("error: no uploaded artifact hash records found in log", file=sys.stderr)
        return 1

    payload = build_payload(args.run_id, run_url, head_sha, records)
    if args.format == "markdown":
        rendered = format_markdown(payload)
    else:
        rendered = json.dumps(payload, indent=2) + "\n"

    if args.out:
        out_path = Path(args.out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(rendered, encoding="utf-8")
        print(f"wrote {args.out}")
    else:
        print(rendered, end="")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
