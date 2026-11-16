#!/usr/bin/env python3
"""Deterministic Python wheelhouse manager with SHA-256 manifest verification for CI."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
import sys
from pathlib import Path
from typing import Any


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        while True:
            chunk = fh.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest().lower()


def normalized_requirements_sha256(requirements_path: Path) -> str:
    lines = [
        line.strip()
        for line in requirements_path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    payload = "\n".join(lines).encode("utf-8")
    return hashlib.sha256(payload).hexdigest().lower()


def compute_fingerprint(requirements_path: Path) -> dict[str, Any]:
    sys_name = platform.system().lower() or "unknown"
    arch = platform.machine().lower() or "unknown"
    py_tag = f"py{sys.version_info.major}.{sys.version_info.minor}-{sys.implementation.name}"
    req_sha = normalized_requirements_sha256(requirements_path)
    key = f"wheelhouse-{sys_name}-{arch}-{py_tag}-{req_sha[:16]}"
    return {
        "schemaVersion": 1,
        "key": key,
        "system": sys_name,
        "arch": arch,
        "pythonTag": py_tag,
        "requirementsSha256": req_sha,
    }


def write_manifest(wheelhouse_dir: Path, requirements_path: Path) -> dict[str, Any]:
    fp = compute_fingerprint(requirements_path)
    wheels: list[dict[str, Any]] = []
    for whl in sorted(wheelhouse_dir.glob("*.whl"), key=lambda p: p.name):
        wheels.append(
            {
                "filename": whl.name,
                "sizeBytes": whl.stat().st_size,
                "sha256": sha256_file(whl),
            }
        )
    if not wheels:
        raise ValueError(f"No .whl files found in {wheelhouse_dir}")

    manifest = {
        "schemaVersion": 1,
        "fingerprint": fp,
        "requirementsSha256": fp["requirementsSha256"],
        "wheels": wheels,
    }
    manifest_path = wheelhouse_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return manifest


def verify_wheelhouse(
    wheelhouse_dir: Path,
    requirements_path: Path,
    *,
    strict_fingerprint: bool = True,
) -> dict[str, Any]:
    manifest_path = wheelhouse_dir / "manifest.json"
    if not manifest_path.is_file():
        raise FileNotFoundError(f"Wheelhouse manifest missing: {manifest_path}")

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    expected_fp = compute_fingerprint(requirements_path)
    if manifest.get("requirementsSha256") != expected_fp["requirementsSha256"]:
        raise ValueError("Wheelhouse requirementsSha256 mismatch against ci/apple-tools-requirements.txt")
    if strict_fingerprint and manifest.get("fingerprint", {}).get("key") != expected_fp["key"]:
        raise ValueError(
            f"Wheelhouse fingerprint mismatch: expected {expected_fp['key']}, "
            f"got {manifest.get('fingerprint', {}).get('key')}"
        )

    listed_wheels = manifest.get("wheels")
    if not isinstance(listed_wheels, list) or not listed_wheels:
        raise ValueError("Wheelhouse manifest contains no wheels")

    actual_names = {p.name for p in wheelhouse_dir.glob("*.whl")}
    manifest_names: set[str] = set()
    for entry in listed_wheels:
        name = entry.get("filename")
        expected_sha = str(entry.get("sha256", "")).lower()
        expected_size = int(entry.get("sizeBytes", -1))
        if not name or not expected_sha:
            raise ValueError(f"Malformed wheel manifest entry: {entry}")
        manifest_names.add(name)
        whl_path = wheelhouse_dir / name
        if not whl_path.is_file():
            raise FileNotFoundError(f"Missing wheel file listed in manifest: {name}")
        actual_size = whl_path.stat().st_size
        if actual_size != expected_size:
            raise ValueError(f"Wheel size mismatch for {name}: expected {expected_size}, got {actual_size}")
        actual_sha = sha256_file(whl_path)
        if actual_sha != expected_sha:
            raise ValueError(f"Wheel SHA-256 mismatch for {name}: expected {expected_sha}, got {actual_sha}")

    extra_wheels = actual_names - manifest_names
    if extra_wheels:
        raise ValueError(f"Unexpected unlisted wheels in wheelhouse: {sorted(extra_wheels)}")

    return {
        "verified": True,
        "key": expected_fp["key"],
        "wheelCount": len(listed_wheels),
    }


def build_wheelhouse(
    wheelhouse_dir: Path,
    requirements_path: Path,
    python_exe: str = sys.executable,
) -> dict[str, Any]:
    wheelhouse_dir.mkdir(parents=True, exist_ok=True)
    for stale in wheelhouse_dir.glob("*.whl"):
        stale.unlink()
    subprocess.run(
        [
            python_exe,
            "-m",
            "pip",
            "wheel",
            "--requirement",
            str(requirements_path),
            "--wheel-dir",
            str(wheelhouse_dir),
        ],
        check=True,
    )
    return write_manifest(wheelhouse_dir, requirements_path)


def install_from_wheelhouse(
    wheelhouse_dir: Path,
    requirements_path: Path,
    python_exe: str = sys.executable,
) -> dict[str, Any]:
    verification = verify_wheelhouse(wheelhouse_dir, requirements_path, strict_fingerprint=True)
    subprocess.run(
        [
            python_exe,
            "-m",
            "pip",
            "install",
            "--no-index",
            "--find-links",
            str(wheelhouse_dir),
            "--only-binary=:all:",
            "--requirement",
            str(requirements_path),
        ],
        check=True,
    )
    return verification


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Deterministic offline Python wheelhouse manager")
    sub = parser.add_subparsers(dest="command", required=True)

    for cmd_name in ("fingerprint", "build", "verify", "install"):
        sub_p = sub.add_parser(cmd_name)
        sub_p.add_argument("--requirements", required=True, type=Path)
        if cmd_name != "fingerprint":
            sub_p.add_argument("--wheelhouse-dir", required=True, type=Path)
        if cmd_name in ("build", "install"):
            sub_p.add_argument("--python-exe", default=sys.executable)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.command == "fingerprint":
        print(json.dumps(compute_fingerprint(args.requirements), indent=2))
        return 0
    if args.command == "build":
        print(json.dumps(build_wheelhouse(args.wheelhouse_dir, args.requirements, args.python_exe), indent=2))
        return 0
    if args.command == "verify":
        print(json.dumps(verify_wheelhouse(args.wheelhouse_dir, args.requirements), indent=2))
        return 0
    if args.command == "install":
        print(json.dumps(install_from_wheelhouse(args.wheelhouse_dir, args.requirements, args.python_exe), indent=2))
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
