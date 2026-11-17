#!/usr/bin/env python3
"""Dual-bundle (.app + Widgets .appex) signature & plist verifier for .xcarchive and .ipa."""

from __future__ import annotations

import argparse
import hashlib
import json
import plistlib
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path
from typing import Any, Callable


RunnerFn = Callable[..., subprocess.CompletedProcess[str]]


def compute_file_sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        while True:
            chunk = fh.read(1024 * 1024)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest().lower()


def _default_runner(cmd: list[str], *, check: bool = True, capture_output: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, check=check, capture_output=capture_output, text=True)


def _extract_entitlements_plist(raw_codesign_output: str) -> dict[str, Any]:
    start = raw_codesign_output.find("<?xml")
    if start == -1:
        start = raw_codesign_output.find("<plist")
    end = raw_codesign_output.find("</plist>")
    if start == -1 or end == -1:
        raise ValueError("Could not find entitlements XML plist in codesign output")
    xml_bytes = raw_codesign_output[start : end + len("</plist>")].encode("utf-8")
    parsed = plistlib.loads(xml_bytes)
    if not isinstance(parsed, dict):
        raise ValueError("Extracted entitlements are not a dictionary")
    return parsed


def _verify_single_bundle(
    bundle_dir: Path,
    *,
    expected_bundle_id: str,
    expected_team_id: str,
    expected_cert_sha256: str,
    expected_app_group: str,
    expected_marketing_version: str,
    expected_build_number: str,
    forbid_coreffi_framework: bool,
    runner: RunnerFn,
) -> dict[str, Any]:
    info_plist_path = bundle_dir / "Info.plist"
    if not info_plist_path.is_file():
        raise FileNotFoundError(f"Missing Info.plist in bundle: {bundle_dir}")

    with info_plist_path.open("rb") as fh:
        info = plistlib.load(fh)

    actual_bundle_id = str(info.get("CFBundleIdentifier", ""))
    actual_version = str(info.get("CFBundleShortVersionString", ""))
    actual_build = str(info.get("CFBundleVersion", ""))
    if actual_bundle_id != expected_bundle_id:
        raise ValueError(f"Bundle ID mismatch at {bundle_dir}: expected {expected_bundle_id!r}, got {actual_bundle_id!r}")
    if actual_version != expected_marketing_version:
        raise ValueError(
            f"CFBundleShortVersionString mismatch at {bundle_dir}: expected {expected_marketing_version!r}, got {actual_version!r}"
        )
    if actual_build != expected_build_number:
        raise ValueError(f"CFBundleVersion mismatch at {bundle_dir}: expected {expected_build_number!r}, got {actual_build!r}")

    if forbid_coreffi_framework:
        frameworks_dir = bundle_dir / "Frameworks"
        if frameworks_dir.is_dir():
            ffi_items = [p.name for p in frameworks_dir.iterdir() if "TokenVaneCoreFFI" in p.name]
            if ffi_items:
                raise ValueError(f"Widget extension illegally embeds CoreFFI framework: {ffi_items}")

    runner(["codesign", "--verify", "--deep", "--strict", "--verbose=4", str(bundle_dir)], check=True, capture_output=True)
    dv_proc = runner(["codesign", "-dvv", str(bundle_dir)], check=True, capture_output=True)
    dv_text = (dv_proc.stderr or "") + "\n" + (dv_proc.stdout or "")
    if f"TeamIdentifier={expected_team_id}" not in dv_text:
        raise ValueError(f"codesign TeamIdentifier mismatch for {bundle_dir}: expected TeamIdentifier={expected_team_id}")

    with tempfile.TemporaryDirectory(prefix="tt-cert-check-") as cert_dir:
        cert_prefix = Path(cert_dir) / "cert"
        runner(["codesign", "-d", f"--extract-certificates={cert_prefix}", str(bundle_dir)], check=True, capture_output=True)
        leaf_cert = Path(f"{cert_prefix}0")
        if not leaf_cert.is_file():
            raise FileNotFoundError(f"codesign did not extract leaf certificate for {bundle_dir}")
        actual_cert_sha = hashlib.sha256(leaf_cert.read_bytes()).hexdigest().lower()
        if actual_cert_sha != expected_cert_sha256.lower():
            raise ValueError(
                f"Signing certificate SHA-256 mismatch for {bundle_dir}: "
                f"expected {expected_cert_sha256.lower()}, got {actual_cert_sha}"
            )

    ent_proc = runner(["codesign", "-d", "--entitlements", ":-", str(bundle_dir)], check=True, capture_output=True)
    entitlements = _extract_entitlements_plist((ent_proc.stdout or "") + "\n" + (ent_proc.stderr or ""))

    if entitlements.get("get-task-allow") is True:
        raise ValueError(f"Bundle {bundle_dir} unexpectedly has get-task-allow=true")

    expected_app_id = f"{expected_team_id}.{expected_bundle_id}"
    if entitlements.get("application-identifier") != expected_app_id:
        raise ValueError(
            f"Entitlement application-identifier mismatch for {bundle_dir}: "
            f"expected {expected_app_id!r}, got {entitlements.get('application-identifier')!r}"
        )
    groups = entitlements.get("com.apple.security.application-groups") or []
    if expected_app_group not in groups:
        raise ValueError(f"Entitlement application-groups missing {expected_app_group!r} in {bundle_dir}")

    return {
        "bundleId": actual_bundle_id,
        "marketingVersion": actual_version,
        "buildNumber": actual_build,
        "certificateSha256": expected_cert_sha256.lower(),
        "codesignVerified": True,
    }


def verify_signing_artifacts(
    *,
    xcarchive_path: Path,
    ipa_path: Path,
    bundle_base_id: str,
    widgets_bundle_id: str,
    app_group_id: str,
    team_id: str,
    certificate_sha256: str,
    marketing_version: str,
    build_number: str,
    runner: RunnerFn = _default_runner,
) -> dict[str, Any]:
    if not xcarchive_path.is_dir():
        raise FileNotFoundError(f"Missing .xcarchive directory: {xcarchive_path}")
    if not ipa_path.is_file():
        raise FileNotFoundError(f"Missing .ipa archive file: {ipa_path}")

    archive_app = xcarchive_path / "Products" / "Applications" / "TokenVane.app"
    archive_widgets = archive_app / "PlugIns" / "TokenVaneWidgets.appex"

    archive_app_info = _verify_single_bundle(
        archive_app,
        expected_bundle_id=bundle_base_id,
        expected_team_id=team_id,
        expected_cert_sha256=certificate_sha256,
        expected_app_group=app_group_id,
        expected_marketing_version=marketing_version,
        expected_build_number=build_number,
        forbid_coreffi_framework=False,
        runner=runner,
    )
    archive_widgets_info = _verify_single_bundle(
        archive_widgets,
        expected_bundle_id=widgets_bundle_id,
        expected_team_id=team_id,
        expected_cert_sha256=certificate_sha256,
        expected_app_group=app_group_id,
        expected_marketing_version=marketing_version,
        expected_build_number=build_number,
        forbid_coreffi_framework=True,
        runner=runner,
    )

    with tempfile.TemporaryDirectory(prefix="tt-ipa-verify-") as tmpdir:
        if sys.platform == "darwin":
            # Preserve executable modes and symlinks for codesign --strict.
            subprocess.run(["ditto", "-x", "-k", str(ipa_path), tmpdir], check=True)
        else:
            with zipfile.ZipFile(ipa_path, "r") as zf:
                zf.extractall(tmpdir)
        ipa_app = Path(tmpdir) / "Payload" / "TokenVane.app"
        ipa_widgets = ipa_app / "PlugIns" / "TokenVaneWidgets.appex"
        ipa_app_info = _verify_single_bundle(
            ipa_app,
            expected_bundle_id=bundle_base_id,
            expected_team_id=team_id,
            expected_cert_sha256=certificate_sha256,
            expected_app_group=app_group_id,
            expected_marketing_version=marketing_version,
            expected_build_number=build_number,
            forbid_coreffi_framework=False,
            runner=runner,
        )
        ipa_widgets_info = _verify_single_bundle(
            ipa_widgets,
            expected_bundle_id=widgets_bundle_id,
            expected_team_id=team_id,
            expected_cert_sha256=certificate_sha256,
            expected_app_group=app_group_id,
            expected_marketing_version=marketing_version,
            expected_build_number=build_number,
            forbid_coreffi_framework=True,
            runner=runner,
        )

    return {
        "schemaVersion": 1,
        "verified": True,
        "teamId": team_id,
        "certificateSha256": certificate_sha256.lower(),
        "marketingVersion": marketing_version,
        "buildNumber": build_number,
        "appGroupId": app_group_id,
        "bundles": {
            "app": {
                "bundleId": ipa_app_info["bundleId"],
                "xcarchiveVerified": archive_app_info["codesignVerified"],
                "ipaVerified": ipa_app_info["codesignVerified"],
            },
            "widgets": {
                "bundleId": ipa_widgets_info["bundleId"],
                "xcarchiveVerified": archive_widgets_info["codesignVerified"],
                "ipaVerified": ipa_widgets_info["codesignVerified"],
            },
        },
        "ipa": {
            "filename": ipa_path.name,
            "sizeBytes": ipa_path.stat().st_size,
            "sha256": compute_file_sha256(ipa_path),
        },
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Verify iOS .xcarchive and .ipa signing & bundle integrity")
    parser.add_argument("--xcarchive", required=True, type=Path)
    parser.add_argument("--ipa", required=True, type=Path)
    parser.add_argument("--candidate", type=Path, default=Path("ci/testflight-candidate.json"))
    parser.add_argument("--team-id", default=None)
    parser.add_argument("--cert-sha256", default=None)
    parser.add_argument("--output", required=True, type=Path)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    candidate = json.loads(args.candidate.read_text(encoding="utf-8"))
    team_id = args.team_id or candidate["teamId"]
    cert_sha256 = args.cert_sha256 or candidate["certificateSha256"]

    report = verify_signing_artifacts(
        xcarchive_path=args.xcarchive,
        ipa_path=args.ipa,
        bundle_base_id=candidate["bundleBaseId"],
        widgets_bundle_id=candidate["widgetsBundleId"],
        app_group_id=candidate["appGroupId"],
        team_id=team_id,
        certificate_sha256=cert_sha256,
        marketing_version=candidate["marketingVersion"],
        build_number=candidate["buildNumber"],
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
