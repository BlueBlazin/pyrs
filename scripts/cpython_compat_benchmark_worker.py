#!/usr/bin/env python3
"""Run or inventory a unittest module for the CPython compatibility benchmark."""

from __future__ import annotations

import argparse
import contextlib
import importlib
import io
import json
import os
import pathlib
import sys
import time
import traceback
import unittest
from collections import Counter
from typing import Any


SCHEMA_VERSION = "v1"
SUBTEST_DURATION_ATTR = "__pyrs_benchmark_elapsed__"
MAX_DETAIL_CHARS = 16_000


def truncate_detail(detail: str | None) -> str | None:
    if detail is None:
        return None
    if len(detail) <= MAX_DETAIL_CHARS:
        return detail
    return f"{detail[:MAX_DETAIL_CHARS]}...<truncated>"


def payload_for_exception(exc: BaseException) -> dict[str, Any]:
    detail = "".join(traceback.format_exception(type(exc), exc, exc.__traceback__))
    return {
        "type": type(exc).__name__,
        "message": str(exc),
        "detail": truncate_detail(detail),
    }


def event_counts(items: list[dict[str, Any]]) -> dict[str, int]:
    counts = Counter(item["outcome"] for item in items)
    return dict(sorted(counts.items()))


def discover_case_ids(suite: unittest.TestSuite) -> list[str]:
    ids: list[str] = []
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            ids.extend(discover_case_ids(test))
            continue
        if isinstance(test, unittest.loader._FailedTest):  # type: ignore[attr-defined]
            continue
        if isinstance(test, unittest.TestCase):
            ids.append(test.id())
    return ids


def case_id_parts(test_id: str) -> tuple[str, str, str]:
    parts = test_id.split(".")
    if len(parts) < 3:
        return test_id, "", test_id
    return ".".join(parts[:-2]), parts[-2], parts[-1]


def classify_err(test: unittest.case.TestCase, err: tuple[type[BaseException], BaseException, Any]) -> str:
    return "failed" if issubclass(err[0], test.failureException) else "error"


def install_subtest_timing_patch() -> None:
    outcome_type = unittest.case._Outcome  # type: ignore[attr-defined]
    if getattr(outcome_type.testPartExecutor, "__pyrs_benchmark_patch__", False):
        return

    @contextlib.contextmanager
    def patched(self, test_case, subTest=False):  # type: ignore[no-untyped-def]
        old_success = self.success
        self.success = True
        start = time.perf_counter() if subTest else None
        try:
            yield
        except KeyboardInterrupt:
            raise
        except unittest.case.SkipTest as exc:  # type: ignore[attr-defined]
            self.success = False
            unittest.case._addSkip(self.result, test_case, str(exc))  # type: ignore[attr-defined]
        except unittest.case._ShouldStop:  # type: ignore[attr-defined]
            pass
        except BaseException:
            exc_info = sys.exc_info()
            if self.expecting_failure:
                self.expectedFailure = exc_info
            else:
                self.success = False
                if subTest:
                    if start is not None:
                        setattr(test_case, SUBTEST_DURATION_ATTR, time.perf_counter() - start)
                    self.result.addSubTest(test_case.test_case, test_case, exc_info)
                else:
                    unittest.case._addError(self.result, test_case, exc_info)  # type: ignore[attr-defined]
            exc_info = None
        else:
            if subTest and self.success:
                if start is not None:
                    setattr(test_case, SUBTEST_DURATION_ATTR, time.perf_counter() - start)
                self.result.addSubTest(test_case.test_case, test_case, None)
        finally:
            self.success = self.success and old_success

    patched.__pyrs_benchmark_patch__ = True  # type: ignore[attr-defined]
    outcome_type.testPartExecutor = patched


class BenchmarkResult(unittest.TextTestResult):
    """Structured unittest result collector with per-case and per-subtest events."""

    def __init__(self, stream, descriptions, verbosity, *, durations=None):
        super().__init__(stream, descriptions, verbosity, durations=durations)
        self.case_records: list[dict[str, Any]] = []
        self.subtest_records: list[dict[str, Any]] = []
        self.fixture_records: list[dict[str, Any]] = []
        self._case_index_by_id: dict[str, int] = {}
        self._subtest_outcomes_by_parent: dict[str, list[str]] = {}
        self._duration_by_case_id: dict[str, float] = {}

    def startTest(self, test):
        super().startTest(test)
        test_id = test.id()
        module_name, class_name, method_name = case_id_parts(test_id)
        record = {
            "kind": "case",
            "id": test_id,
            "module": module_name,
            "class": class_name,
            "method": method_name,
            "outcome": None,
            "duration_secs": None,
            "has_subtests": False,
            "subtest_outcome_counts": {},
            "detail": None,
        }
        self._case_index_by_id[test_id] = len(self.case_records)
        self.case_records.append(record)

    def stopTest(self, test):
        test_id = test.id()
        index = self._case_index_by_id.get(test_id)
        if index is not None:
            record = self.case_records[index]
            outcomes = self._subtest_outcomes_by_parent.get(test_id, [])
            if outcomes:
                record["has_subtests"] = True
                record["subtest_outcome_counts"] = dict(sorted(Counter(outcomes).items()))
            if record["outcome"] is None:
                if "error" in outcomes:
                    record["outcome"] = "error"
                elif "failed" in outcomes:
                    record["outcome"] = "failed"
                elif outcomes:
                    record["outcome"] = "passed"
                else:
                    record["outcome"] = "passed"
            record["duration_secs"] = self._duration_by_case_id.get(test_id)
        super().stopTest(test)

    def addDuration(self, test, elapsed):
        super().addDuration(test, elapsed)
        self._duration_by_case_id[test.id()] = round(float(elapsed), 6)

    def _mark_case(self, test, outcome: str, detail: str | None = None) -> None:
        index = self._case_index_by_id.get(test.id())
        if index is None:
            self.fixture_records.append(
                {
                    "kind": "fixture",
                    "id": str(test),
                    "outcome": outcome,
                    "detail": truncate_detail(detail),
                }
            )
            return
        record = self.case_records[index]
        record["outcome"] = outcome
        if detail:
            record["detail"] = truncate_detail(detail)

    def addSuccess(self, test):
        super().addSuccess(test)
        self._mark_case(test, "passed")

    def addSkip(self, test, reason):
        super().addSkip(test, reason)
        self._mark_case(test, "skipped", reason)

    def addExpectedFailure(self, test, err):
        super().addExpectedFailure(test, err)
        self._mark_case(test, "expected_failure", self.expectedFailures[-1][1])

    def addUnexpectedSuccess(self, test):
        super().addUnexpectedSuccess(test)
        self._mark_case(test, "unexpected_success")

    def addFailure(self, test, err):
        before = len(self.failures)
        super().addFailure(test, err)
        detail = self.failures[before][1] if len(self.failures) > before else None
        self._mark_case(test, "failed", detail)

    def addError(self, test, err):
        before = len(self.errors)
        super().addError(test, err)
        detail = self.errors[before][1] if len(self.errors) > before else None
        self._mark_case(test, "error", detail)

    def addSubTest(self, test, subtest, err):
        before_failures = len(self.failures)
        before_errors = len(self.errors)
        super().addSubTest(test, subtest, err)

        if err is None:
            outcome = "passed"
            detail = None
        else:
            outcome = classify_err(subtest, err)
            if outcome == "failed" and len(self.failures) > before_failures:
                detail = self.failures[before_failures][1]
            elif outcome == "error" and len(self.errors) > before_errors:
                detail = self.errors[before_errors][1]
            else:
                detail = None

        duration = getattr(subtest, SUBTEST_DURATION_ATTR, None)
        if duration is not None:
            duration = round(float(duration), 6)
        parent_id = test.id()
        self._subtest_outcomes_by_parent.setdefault(parent_id, []).append(outcome)
        self.subtest_records.append(
            {
                "kind": "subtest",
                "id": subtest.id(),
                "parent_id": parent_id,
                "module": case_id_parts(parent_id)[0],
                "class": case_id_parts(parent_id)[1],
                "method": case_id_parts(parent_id)[2],
                "outcome": outcome,
                "duration_secs": duration,
                "detail": truncate_detail(detail),
            }
        )


def apply_sys_path(paths: list[str]) -> None:
    for raw in reversed(paths):
        path = str(pathlib.Path(raw).resolve())
        if path not in sys.path:
            sys.path.insert(0, path)


def configure_test_support() -> None:
    try:
        from test import support as test_support
    except Exception:
        return
    test_support.use_resources = {}
    test_support.verbose = 0
    test_support.failfast = False
    try:
        from test.libregrtest.setup import setup_process
    except Exception:
        return
    try:
        setup_process()
    except Exception:
        # CPython's harness does environment tuning that pyrs may not implement yet.
        # The benchmark worker only needs resource defaults, not a perfect regrtest shell.
        return


def load_suite(module_name: str) -> tuple[unittest.TestSuite | None, dict[str, Any] | None]:
    loader = unittest.defaultTestLoader
    try:
        suite = loader.loadTestsFromName(module_name)
    except unittest.SkipTest as exc:
        return None, {
            "status": "host_skip",
            "reason": str(exc),
            "error": {
                "type": type(exc).__name__,
                "message": str(exc),
                "detail": None,
            },
        }
    except BaseException as exc:
        return None, {
            "status": "load_error",
            "reason": str(exc),
            "error": payload_for_exception(exc),
        }
    loader_errors = list(getattr(loader, "errors", []))
    if loader_errors:
        return suite, {
            "status": "load_error",
            "reason": "unittest loader errors",
            "error": {
                "type": "LoaderError",
                "message": "unittest loader errors",
                "detail": truncate_detail("\n\n".join(loader_errors)),
            },
        }
    return suite, None


def inventory_payload(module_name: str, suite: unittest.TestSuite | None, load_state: dict[str, Any] | None) -> dict[str, Any]:
    case_ids = [] if suite is None else discover_case_ids(suite)
    return {
        "schema_version": SCHEMA_VERSION,
        "mode": "inventory",
        "module": module_name,
        "status": load_state["status"] if load_state is not None else "ok",
        "case_ids": case_ids,
        "case_count": len(case_ids),
        "load_state": load_state,
        "interpreter": {
            "executable": sys.executable,
            "implementation": sys.implementation.name,
            "version": sys.version.split()[0],
        },
    }


def run_payload(module_name: str, suite: unittest.TestSuite | None, load_state: dict[str, Any] | None) -> dict[str, Any]:
    base = inventory_payload(module_name, suite, load_state)
    base["mode"] = "run"
    if suite is None or load_state is not None:
        base["results"] = {
            "tests_run": 0,
            "case_records": [],
            "subtest_records": [],
            "fixture_records": [],
            "case_outcomes": {},
            "subtest_outcomes": {},
            "fixture_outcomes": {},
        }
        return base

    stream = io.StringIO()
    runner = unittest.TextTestRunner(
        stream=stream,
        verbosity=0,
        failfast=False,
        buffer=True,
        resultclass=BenchmarkResult,
    )
    started = time.perf_counter()
    result: BenchmarkResult = runner.run(suite)  # type: ignore[assignment]
    elapsed = round(time.perf_counter() - started, 6)

    fixture_failure = any(
        event["outcome"] in {"error", "failed", "unexpected_success"}
        for event in result.fixture_records
    )
    case_failure = any(
        record["outcome"] in {"error", "failed", "unexpected_success"}
        for record in result.case_records
    )
    subtest_failure = any(
        record["outcome"] in {"error", "failed", "unexpected_success"}
        for record in result.subtest_records
    )

    if fixture_failure or case_failure or subtest_failure:
        status = "failed"
    elif result.case_records and all(record["outcome"] == "skipped" for record in result.case_records):
        status = "skipped"
    else:
        status = "passed"

    base["status"] = status
    base["runner_output_tail"] = truncate_detail(stream.getvalue()[-4000:])
    base["elapsed_secs"] = elapsed
    base["results"] = {
        "tests_run": result.testsRun,
        "case_records": result.case_records,
        "subtest_records": result.subtest_records,
        "fixture_records": result.fixture_records,
        "case_outcomes": event_counts(result.case_records),
        "subtest_outcomes": event_counts(result.subtest_records),
        "fixture_outcomes": event_counts(result.fixture_records),
    }
    return base


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Inventory or run a unittest module for the CPython compatibility benchmark")
    parser.add_argument("--mode", choices=("inventory", "run"), required=True)
    parser.add_argument("--module", required=True, help="Absolute module name to load (for example test.test_json)")
    parser.add_argument(
        "--sys-path",
        action="append",
        default=[],
        help="Prepend an import root to sys.path before loading the module",
    )
    parser.add_argument("--out", default=None, help="Optional JSON output path")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    install_subtest_timing_patch()
    apply_sys_path(args.sys_path)
    configure_test_support()
    suite, load_state = load_suite(args.module)
    if args.mode == "inventory":
        payload = inventory_payload(args.module, suite, load_state)
    else:
        payload = run_payload(args.module, suite, load_state)

    encoded = json.dumps(payload, indent=2) + "\n"
    if args.out:
        out_path = pathlib.Path(args.out)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(encoded, encoding="utf-8")
    else:
        sys.stdout.write(encoded)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except SystemExit as exc:
        raise exc
    except Exception as exc:
        payload = {
            "schema_version": SCHEMA_VERSION,
            "mode": "worker_crash",
            "status": "worker_crash",
            "error": payload_for_exception(exc),
        }
        sys.stdout.write(json.dumps(payload, indent=2) + "\n")
        raise SystemExit(0)
