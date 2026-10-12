#!/usr/bin/env python3
"""Run the vendored renderer test without resolving the whole Zed workspace."""

import argparse
import copy
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


def inline(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return json.dumps(value)
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{json.dumps(key)} = {inline(item)}" for key, item in value.items()) + " }"
    if isinstance(value, list):
        return "[" + ", ".join(map(inline, value)) + "]"
    return str(value)


def materialize_dependencies(dependencies, inherited, workspace):
    resolved = {}
    for name, value in dependencies.items():
        value = copy.deepcopy(value)
        if isinstance(value, dict) and value.pop("workspace", False):
            base = copy.deepcopy(inherited[name])
            if isinstance(base, str):
                base = {"version": base}
            features = base.get("features", []) + value.pop("features", [])
            base.update(value)
            if features:
                base["features"] = list(dict.fromkeys(features))
            value = base
        if isinstance(value, dict) and "path" in value:
            value["path"] = str((workspace / value["path"]).resolve())
        resolved[name] = value
    return resolved


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--online", action="store_true")
    parser.add_argument("--log-file", type=Path, default=Path("/tmp/dojo-optimization/gpu-resource-test.log"))
    args = parser.parse_args()
    repo = args.repo.resolve()
    workspace = repo / "vendor/zed"
    crate = workspace / "crates/gpui_wgpu"
    authoritative = tomllib.loads((workspace / "Cargo.toml").read_text())
    inherited = authoritative["workspace"]
    original = tomllib.loads((crate / "Cargo.toml").read_text())
    package = copy.deepcopy(original["package"])
    for name, value in list(package.items()):
        if isinstance(value, dict) and value.get("workspace"):
            package[name] = inherited["package"][name]
    lines = ["[workspace]", 'resolver = "2"', "", "[package]"]
    lines += [f"{json.dumps(key)} = {inline(value)}" for key, value in package.items()]
    lines += ["", "[lib]", f"path = {inline(str(crate / original['lib']['path']))}", "doctest = false"]

    def table(name, values):
        lines.extend(["", "[" + ".".join(map(json.dumps, name)) + "]"])
        lines.extend(f"{json.dumps(key)} = {inline(value)}" for key, value in values.items())

    table(["features"], original["features"])
    for section in ["dependencies", "dev-dependencies"]:
        table([section], materialize_dependencies(original[section], inherited["dependencies"], workspace))
    for target, sections in original.get("target", {}).items():
        for section, values in sections.items():
            table(["target", target, section], materialize_dependencies(values, inherited["dependencies"], workspace))
    for name, values in inherited.get("lints", {}).items():
        table(["lints", name], values)
    # This optional capture dependency also enters GPUI's standalone test lockfile.
    table(["patch", "crates-io"], {
        "windows-capture": authoritative["patch"]["crates-io"]["windows-capture"],
    })
    for name, values in tomllib.loads((repo / "Cargo.toml").read_text()).get("profile", {}).items():
        for package_name, options in values.get("package", {}).items():
            table(["profile", name, "package", package_name], options)
    temporary = Path(tempfile.mkdtemp(prefix="dojo-renderer-test-"))
    manifest = temporary / "Cargo.toml"
    manifest.write_text("\n".join(lines) + "\n")
    tomllib.loads(manifest.read_text())
    shutil.copyfile(repo / "Cargo.lock", temporary / "Cargo.lock")
    target = (args.target_dir or repo / "target").resolve()
    command = [
        "devenv", "shell", "--", "env", f"CARGO_TARGET_DIR={target}", "CARGO_BUILD_JOBS=2",
        "cargo", "test", *([] if args.online else ["--offline"]), "--manifest-path", str(manifest), "--lib",
        "wgpu_renderer::tests::headless_path_resources_are_lazy_and_survive_scene_changes",
        "--", "--exact", "--nocapture",
    ]
    print(json.dumps({"manifest": str(manifest), "source": str(crate / original["lib"]["path"]), "command": command}), flush=True)
    if not args.prepare_only:
        args.log_file.parent.mkdir(parents=True, exist_ok=True)
        with args.log_file.open("w") as output:
            result = subprocess.run(command, cwd=repo, stdout=output, stderr=subprocess.STDOUT)
        print("\n".join(args.log_file.read_text().splitlines()[-40:]))
        print(json.dumps({"exit_code": result.returncode, "log": str(args.log_file)}))
        return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
