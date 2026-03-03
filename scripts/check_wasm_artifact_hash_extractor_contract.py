#!/usr/bin/env python3
"""Fixture-backed contract checks for wasm CI artifact hash extraction helper."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
EXTRACT_SCRIPT = REPO_ROOT / "scripts" / "extract_wasm_ci_artifact_hashes.py"
FIXTURE_LOG = (
    REPO_ROOT / "tests" / "fixtures" / "wasm_artifact_hashes" / "sample_workflow_run.log"
)


def run_extractor(args: list[str]) -> str:
    result = subprocess.run(
        [sys.executable, str(EXTRACT_SCRIPT), *args],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        stderr = result.stderr.strip()
        stdout = result.stdout.strip()
        details = stderr or stdout or "unknown extractor failure"
        raise RuntimeError(details)
    return result.stdout


def assert_true(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def main() -> int:
    if not FIXTURE_LOG.is_file():
        print(f"wasm artifact-hash extractor contract failed: missing fixture {FIXTURE_LOG}")
        return 1

    run_url = "https://github.com/BlueBlazin/pyrs/actions/runs/22595444066"
    head_sha = "85d4da15cf300793fcb5538f3bcf06f37327f92c"

    with tempfile.TemporaryDirectory() as tmpdir:
        tmp_path = Path(tmpdir)
        json_out = tmp_path / "hashes.json"
        markdown_out = tmp_path / "hashes.md"

        run_extractor(
            [
                "--log-file",
                str(FIXTURE_LOG),
                "--run-url",
                run_url,
                "--head-sha",
                head_sha,
                "--format",
                "json",
                "--out",
                str(json_out),
            ]
        )

        payload = json.loads(json_out.read_text(encoding="utf-8"))
        assert_true(payload.get("run_id") == "22595444066", "expected run_id inference from run URL")
        assert_true(payload.get("run_url") == run_url, "expected run_url to be preserved")
        assert_true(payload.get("head_sha") == head_sha, "expected head_sha to be preserved")
        assert_true(payload.get("artifact_count") == 2, "expected exactly 2 artifacts from fixture log")

        artifacts = payload.get("artifacts")
        assert_true(isinstance(artifacts, list), "expected artifacts to be a list")
        by_name = {entry.get("name"): entry for entry in artifacts if isinstance(entry, dict)}
        expected = {
            "wasm-contract-artifacts": {
                "artifact_id": "1001",
                "sha256": "a" * 64,
            },
            "wasm-evidence-pack": {
                "artifact_id": "1002",
                "sha256": "b" * 64,
            },
        }
        assert_true(set(by_name.keys()) == set(expected.keys()), "artifact name set mismatch")
        for name, expected_entry in expected.items():
            observed = by_name[name]
            assert_true(
                observed.get("artifact_id") == expected_entry["artifact_id"],
                f"unexpected artifact_id for {name}",
            )
            assert_true(
                observed.get("sha256") == expected_entry["sha256"],
                f"unexpected sha256 for {name}",
            )

        run_extractor(
            [
                "--log-file",
                str(FIXTURE_LOG),
                "--run-url",
                run_url,
                "--head-sha",
                head_sha,
                "--format",
                "markdown",
                "--out",
                str(markdown_out),
            ]
        )
        rendered = markdown_out.read_text(encoding="utf-8")
        assert_true(
            "- workflow run: [22595444066](https://github.com/BlueBlazin/pyrs/actions/runs/22595444066)"
            in rendered,
            "markdown output missing workflow run line",
        )
        assert_true(
            f"- head commit: `{head_sha}`" in rendered,
            "markdown output missing head commit line",
        )
        assert_true(
            "  - `wasm-contract-artifacts`" in rendered
            and "  - `wasm-evidence-pack`" in rendered,
            "markdown output missing artifact list rows",
        )

    print("wasm artifact-hash extractor contract passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
