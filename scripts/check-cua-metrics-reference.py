#!/usr/bin/env python3
"""Optional development reference check. Production and Rust tests do not use Python."""
import ast
import builtins
import contextlib
import functools
import hashlib
import os
import tarfile
import tempfile
import tomllib
import io
import json
import logging
import operator
from pathlib import Path
import re
import subprocess
import sys

REVISION = "88aff7e5424e9ed45b93ded85eb4111472ca5190"
REFERENCE_PYTHON_VERSION = (3, 14, 8)
if sys.version_info[:3] != REFERENCE_PYTHON_VERSION:
    raise SystemExit("Reference fixture generation requires pinned CPython 3.14.8")
PREFIX = "libs/cua-bench/tasks/winarena_adapter/evaluators/metrics/"
source = Path(sys.argv[1])
root = Path(__file__).resolve().parent.parent
namespace = {
    "operator": operator, "functools": functools, "re": re,
    "builtins": builtins, "json": json, "logger": logging.getLogger("reference"),
}
selected = {
    "check_include_exclude", "literal_match", "is_in_list",
    "check_direct_json_object", "check_json", "_match_value_to_rule",
    "_match_record", "compare_python_pure_text", "fuzzy_match",
}
for filename in ("utils.py", "general.py"):
    raw = subprocess.check_output(["git", "-C", str(source), "show", f"{REVISION}:{PREFIX}{filename}"])
    module = ast.parse(raw)
    functions = [node for node in module.body if isinstance(node, ast.FunctionDef) and node.name in selected]
    for function in functions:
        function.returns = None
        for argument in function.args.args:
            argument.annotation = None
        for node in ast.walk(function):
            if isinstance(node, ast.AnnAssign):
                node.annotation = ast.Name(id="object", ctx=ast.Load())
    exec(compile(ast.fix_missing_locations(ast.Module(body=functions, type_ignores=[])), filename, "exec"), namespace)


reference_dependency = None
reference_package = None
if "--rapidfuzz-archive" in sys.argv:
    archive_path = Path(sys.argv[sys.argv.index("--rapidfuzz-archive") + 1])
    lock = tomllib.loads(subprocess.check_output(["git", "-C", str(source), "show", f"{REVISION}:libs/cua-bench/uv.lock"]).decode())
    package = next(package for package in lock["package"] if package["name"] == "rapidfuzz")
    archive_bytes = archive_path.read_bytes()
    digest = hashlib.sha256(archive_bytes).hexdigest()
    if len(archive_bytes) != package["sdist"]["size"] or "sha256:" + digest != package["sdist"]["hash"]:
        raise SystemExit("RapidFuzz source archive differs from pinned upstream lock")
    reference_package = tempfile.TemporaryDirectory(prefix="dojo-metrics-reference-")
    prefix = f"rapidfuzz-{package['version']}/src/"
    with tarfile.open(archive_path) as archive:
        for member in archive.getmembers():
            if not member.isfile() or not member.name.startswith(prefix + "rapidfuzz/") or not member.name.endswith(".py"):
                continue
            relative = Path(member.name.removeprefix(prefix))
            if relative.is_absolute() or ".." in relative.parts:
                raise SystemExit("Invalid reference archive path")
            destination = Path(reference_package.name) / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(archive.extractfile(member).read())
    os.environ["RAPIDFUZZ_IMPLEMENTATION"] = "python"
    sys.path.insert(0, reference_package.name)
    from rapidfuzz import fuzz
    namespace["fuzz"] = fuzz
    reference_dependency = {"name": "rapidfuzz", "version": package["version"], "source_sha256": digest, "implementation": "pinned pure Python reference, development only"}


def translate_rule(rule):
    comparison = rule["comparison"]
    method = comparison["method"]
    result = {"key": rule["key"], "method": method, "ref": comparison.get("reference")}
    if method == "approx":
        result["method"] = "approx:" + str(comparison["threshold"])
    if method == "range":
        result.update(method="range." + ("e" if comparison["minimum_inclusive"] else "t") + ("e" if comparison["maximum_inclusive"] else "t"), ref=[comparison["minimum"], comparison["maximum"]])
    return result


def reference(case):
    metric = case["metric"]
    value = case["evidence"]
    kind = metric["kind"]
    if kind == "include_exclude":
        return namespace["check_include_exclude"](value, metric)
    if kind in ("literal_string", "literal_list"):
        return namespace["literal_match"](value, metric["expected"], type="str" if kind == "literal_string" else "list", ignore_case=metric.get("ignore_case", False))
    if kind == "membership":
        return namespace["is_in_list"](value, metric)
    if kind in ("json_subset", "json_inclusion"):
        return namespace["check_direct_json_object"](value, {**metric, "expect_in_result": kind == "json_inclusion"})
    if kind == "json_rules":
        namespace["open"] = lambda name: io.StringIO(json.dumps(value))
        rules = {key: [translate_rule(rule) for rule in metric.get(key, [])] for key in ("expect", "unexpect")}
        return namespace["check_json"]("evidence.json", rules)
    if kind == "records":
        return float(all(any(namespace["_match_record"](rule, row) for row in value) for rule in metric.get("expect", [])) and all(not namespace["_match_record"](rule, row) for rule in metric.get("unexpect", []) for row in value))
    if kind == "fuzzy_text" and reference_dependency is not None:
        return namespace["fuzzy_match"](value, metric)
    if kind == "text_equal":
        if not metric.get("ignore_whitespace", False):
            return float(value == metric["expected"])
        namespace["open"] = lambda name, mode: io.StringIO(value if name == "actual" else metric["expected"])
        return namespace["compare_python_pure_text"]("actual", "expected")
    return None


fixture = root / "crates/dojo-computer-recording/fixtures/general-metrics.json"
cases = json.loads(fixture.read_text())
results = []
with contextlib.redirect_stdout(io.StringIO()):
    for index, case in enumerate(cases):
        reward = reference(case)
        if reward is not None:
            results.append({"case_index": index, "upstream_reward": reward, "rust_expected_reward": case["reward"], "matches": reward == case["reward"]})
divergences = [result for result in results if not result["matches"]]
if divergences != [{"case_index": 9, "upstream_reward": 1.0, "rust_expected_reward": 0, "matches": False}]:
    raise SystemExit("Unexpected pinned reference divergence: " + json.dumps(divergences))
report = {
    "commit": REVISION,
    "reference_python_version": "3.14.8",
    "method": "Selected pinned function definitions; file getters receive supplied immutable evidence.",
    "fuzzy_reference": reference_dependency,
    "intentional_divergence": "Missing object key does not satisfy an expected null value.",
    "cases": results,
}
destination = fixture.with_name("general-reference-results.json")
serialized = json.dumps(report, indent=2) + "\n"
if "--write" in sys.argv[2:]:
    destination.write_text(serialized)
elif destination.read_text() != serialized:
    raise SystemExit("Reference report changed; review before --write")
print(json.dumps({"compared": len(results), "matched": sum(result["matches"] for result in results), "intentional_divergences": len(divergences)}))
