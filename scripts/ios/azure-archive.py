#!/usr/bin/env python3
"""Provisioning profile validator and ephemeral macOS keychain wrapper (zero-secret logging)."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import platform
import plistlib
import secrets
import shutil
import signal
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Callable

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.serialization import pkcs12


def _ensure_utc(value: dt.datetime) -> dt.datetime:
    if value.tzinfo is None:
        return value.replace(tzinfo=dt.timezone.utc)
    return value.astimezone(dt.timezone.utc)


def decode_mobileprovision_bytes(raw: bytes, *, prefer_security_cms_path: Path | None = None) -> dict[str, Any]:
    if (
        prefer_security_cms_path is not None
        and platform.system() == "Darwin"
        and shutil.which("security") is not None
    ):
        proc = subprocess.run(
            ["security", "cms", "-D", "-i", str(prefer_security_cms_path)],
            capture_output=True,
            check=True,
        )
        return plistlib.loads(proc.stdout)

    start = raw.find(b"<?xml")
    if start == -1:
        start = raw.find(b"<plist")
    end = raw.find(b"</plist>")
    if start == -1 or end == -1:
        raise ValueError("Could not locate XML plist payload inside .mobileprovision data")
    plist_bytes = raw[start : end + len(b"</plist>")]
    parsed = plistlib.loads(plist_bytes)
    if not isinstance(parsed, dict):
        raise ValueError("Decoded provisioning profile is not a plist dictionary")
    return parsed


def validate_profile_dict(
    profile: dict[str, Any],
    *,
    expected_bundle_id: str,
    expected_team_id: str,
    expected_app_group: str,
    expected_cert_sha256: str,
    now_utc: dt.datetime | None = None,
) -> dict[str, Any]:
    if not expected_team_id or expected_team_id == "UNSET":
        raise ValueError("expected_team_id must be set and not UNSET")
    if not expected_cert_sha256 or expected_cert_sha256 == "UNSET":
        raise ValueError("expected_cert_sha256 must be set and not UNSET")

    now = now_utc or dt.datetime.now(dt.timezone.utc)
    expires_raw = profile.get("ExpirationDate")
    if not isinstance(expires_raw, dt.datetime):
        raise ValueError("Provisioning profile missing ExpirationDate")
    expires_utc = _ensure_utc(expires_raw)
    if expires_utc <= now:
        raise ValueError(f"Provisioning profile expired at {expires_utc.isoformat()}")

    if "ProvisionedDevices" in profile:
        raise ValueError("Provisioning profile contains ProvisionedDevices (must be App Store Distribution profile)")

    team_ids = profile.get("TeamIdentifier") or []
    if expected_team_id not in team_ids:
        raise ValueError(f"Profile TeamIdentifier {team_ids!r} does not include {expected_team_id!r}")

    entitlements = profile.get("Entitlements")
    if not isinstance(entitlements, dict):
        raise ValueError("Provisioning profile missing Entitlements dictionary")

    if entitlements.get("get-task-allow") is not False:
        raise ValueError("Provisioning profile must set Entitlements.get-task-allow = false")

    expected_app_id = f"{expected_team_id}.{expected_bundle_id}"
    actual_app_id = entitlements.get("application-identifier")
    if actual_app_id != expected_app_id:
        raise ValueError(f"Profile application-identifier mismatch: expected {expected_app_id!r}, got {actual_app_id!r}")

    app_groups = entitlements.get("com.apple.security.application-groups") or []
    if expected_app_group not in app_groups:
        raise ValueError(f"Profile application-groups {app_groups!r} missing {expected_app_group!r}")

    dev_certs = profile.get("DeveloperCertificates") or []
    cert_hashes: list[str] = []
    for raw_cert in dev_certs:
        if isinstance(raw_cert, (bytes, bytearray)):
            cert_hashes.append(hashlib.sha256(bytes(raw_cert)).hexdigest().lower())
    target_sha = expected_cert_sha256.strip().lower()
    if target_sha not in cert_hashes:
        raise ValueError(f"Profile certificates {cert_hashes!r} do not include expected SHA-256 {target_sha}")

    return {
        "name": str(profile.get("Name", "")),
        "uuid": str(profile.get("UUID", "")),
        "teamId": expected_team_id,
        "applicationIdentifier": actual_app_id,
        "expiresAt": expires_utc.isoformat(),
    }


def _select_legacy_p12_encryption(password_bytes: bytes) -> Any:
    if hasattr(pkcs12, "PBES"):
        return (
            serialization.PrivateFormat.PKCS12.encryption_builder()
            .kdf_rounds(2048)
            .key_cert_algorithm(pkcs12.PBES.PBESv1SHA1And3KeyTripleDESCBC)
            .hmac_hash(hashes.SHA1())
            .build(password_bytes)
        )
    return serialization.BestAvailableEncryption(password_bytes)


def reencode_p12_legacy(
    p12_bytes: bytes,
    passphrase: str,
    expected_cert_sha256: str,
    out_legacy_p12: Path,
) -> tuple[str, str]:
    key, cert, cas = pkcs12.load_key_and_certificates(p12_bytes, passphrase.encode("utf-8"))
    if key is None or cert is None:
        raise ValueError(".p12 bundle must contain both private key and leaf certificate")
    if isinstance(key, rsa.RSAPrivateKey):
        cert_pub = cert.public_key()
        if not isinstance(cert_pub, rsa.RSAPublicKey) or key.public_key().public_numbers() != cert_pub.public_numbers():
            raise ValueError(".p12 private key does not match leaf certificate")

    actual_sha = cert.fingerprint(hashes.SHA256()).hex().lower()
    expected_norm = expected_cert_sha256.strip().lower()
    if expected_norm and expected_norm != "unset" and actual_sha != expected_norm:
        raise ValueError(f".p12 certificate SHA-256 mismatch: expected {expected_norm}, got {actual_sha}")

    one_time_password = secrets.token_hex(24)
    legacy_bytes = pkcs12.serialize_key_and_certificates(
        name=b"TokenTracker Distribution",
        key=key,
        cert=cert,
        cas=cas,
        encryption_algorithm=_select_legacy_p12_encryption(one_time_password.encode("utf-8")),
    )
    out_legacy_p12.parent.mkdir(parents=True, exist_ok=True)
    out_legacy_p12.write_bytes(legacy_bytes)
    if os.name == "posix":
        os.chmod(out_legacy_p12, 0o600)
    return one_time_password, actual_sha


RunnerFn = Callable[..., subprocess.CompletedProcess[str]]


def _default_runner(cmd: list[str], *, check: bool = True, capture_output: bool = False, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(cmd, check=False, capture_output=capture_output, text=True, env=env)
    if check and result.returncode:
        # CalledProcessError includes argv, including security import passwords.
        raise RuntimeError(f"{Path(cmd[0]).name} failed with exit code {result.returncode}")
    return result


def run_with_ephemeral_keychain(
    *,
    p12_path: Path,
    passphrase: str,
    expected_cert_sha256: str,
    child_command: list[str],
    temp_root: Path | None = None,
    runner: RunnerFn = _default_runner,
    install_signal_handlers: bool = True,
) -> int:
    if not child_command:
        raise ValueError("child_command must not be empty")

    base_temp = temp_root or Path(os.environ.get("AGENT_TEMPDIRECTORY") or tempfile.gettempdir())
    work_dir = Path(tempfile.mkdtemp(prefix="tt-apple-kc-", dir=str(base_temp)))
    if os.name == "posix":
        os.chmod(work_dir, 0o700)

    legacy_p12_path = work_dir / "identity.legacy.p12"
    keychain_path = work_dir / "ephemeral.keychain-db"
    keychain_pw = secrets.token_hex(24)

    orig_default_kc: str | None = None
    orig_search_kcs: list[str] = []
    keychain_created = False
    cleaned = False

    def cleanup() -> None:
        nonlocal cleaned
        if cleaned:
            return
        cleaned = True
        try:
            if legacy_p12_path.exists():
                legacy_p12_path.unlink()
        except OSError:
            pass

        if keychain_created:
            if orig_default_kc:
                try:
                    runner(["security", "default-keychain", "-d", "user", "-s", orig_default_kc], check=False, capture_output=True)
                except Exception:
                    pass
            if orig_search_kcs:
                try:
                    runner(["security", "list-keychains", "-d", "user", "-s", *orig_search_kcs], check=False, capture_output=True)
                except Exception:
                    pass
            try:
                runner(["security", "delete-keychain", str(keychain_path)], check=False, capture_output=True)
            except Exception:
                pass
        shutil.rmtree(work_dir, ignore_errors=True)

    prev_handlers: dict[int, Any] = {}
    if install_signal_handlers:
        def _on_signal(signum: int, _frame: Any) -> None:
            cleanup()
            raise SystemExit(128 + signum)

        for sig_name in ("SIGINT", "SIGTERM"):
            sig = getattr(signal, sig_name, None)
            if sig is not None:
                prev_handlers[sig] = signal.getsignal(sig)
                signal.signal(sig, _on_signal)

    try:
        one_time_pw, actual_sha = reencode_p12_legacy(
            p12_path.read_bytes(),
            passphrase,
            expected_cert_sha256,
            legacy_p12_path,
        )
        sys.stderr.write(f"[azure-archive] Verified signing certificate SHA-256: {actual_sha}\n")

        def_proc = runner(["security", "default-keychain", "-d", "user"], check=False, capture_output=True)
        if def_proc.returncode == 0 and def_proc.stdout:
            orig_default_kc = def_proc.stdout.strip().strip('"')

        list_proc = runner(["security", "list-keychains", "-d", "user"], check=False, capture_output=True)
        if list_proc.returncode == 0 and list_proc.stdout:
            orig_search_kcs = [line.strip().strip('"') for line in list_proc.stdout.splitlines() if line.strip()]

        runner(["security", "create-keychain", "-p", keychain_pw, str(keychain_path)], check=True, capture_output=True)
        keychain_created = True
        runner(["security", "set-keychain-settings", "-lut", "3600", str(keychain_path)], check=True, capture_output=True)
        runner(["security", "unlock-keychain", "-p", keychain_pw, str(keychain_path)], check=True, capture_output=True)

        runner(
            [
                "security",
                "import",
                str(legacy_p12_path),
                "-k",
                str(keychain_path),
                "-P",
                one_time_pw,
                "-T",
                "/usr/bin/codesign",
                "-T",
                "/usr/bin/security",
            ],
            check=True,
            capture_output=True,
        )
        legacy_p12_path.unlink()

        runner(
            [
                "security",
                "set-key-partition-list",
                "-S",
                "apple-tool:,apple:,codesign:",
                "-s",
                "-k",
                keychain_pw,
                str(keychain_path),
            ],
            check=True,
            capture_output=True,
        )
        runner(["security", "list-keychains", "-d", "user", "-s", str(keychain_path), *orig_search_kcs], check=True, capture_output=True)
        runner(["security", "default-keychain", "-d", "user", "-s", str(keychain_path)], check=True, capture_output=True)

        child_env = dict(os.environ)
        child_env["TOKENTRACKER_KEYCHAIN_PATH"] = str(keychain_path)
        proc = runner(child_command, check=True, env=child_env)
        return int(proc.returncode)
    finally:
        cleanup()
        for sig, handler in prev_handlers.items():
            signal.signal(sig, handler)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Apple provisioning validator & ephemeral keychain runner")
    sub = parser.add_subparsers(dest="command", required=True)

    val_p = sub.add_parser("validate-profiles", help="Validate App & Widgets Distribution .mobileprovision files")
    val_p.add_argument("--app-profile", required=True, type=Path)
    val_p.add_argument("--widgets-profile", required=True, type=Path)
    val_p.add_argument("--bundle-base-id", default="com.tokentracker.app")
    val_p.add_argument("--app-group", default="group.com.tokentracker.app")
    val_p.add_argument("--team-id", required=True)
    val_p.add_argument("--cert-sha256", required=True)

    kc_p = sub.add_parser("with-keychain", help="Run child command with an ephemeral macOS signing keychain")
    kc_p.add_argument("--p12", required=True, type=Path)
    kc_p.add_argument("--p12-password-env", default="APPLE_P12_PASSWORD")
    kc_p.add_argument("--p12-password-file", type=Path, default=None)
    kc_p.add_argument("--cert-sha256", required=True)
    kc_p.add_argument("child", nargs=argparse.REMAINDER)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.command == "validate-profiles":
        app_dict = decode_mobileprovision_bytes(
            args.app_profile.read_bytes(),
            prefer_security_cms_path=args.app_profile,
        )
        widgets_dict = decode_mobileprovision_bytes(
            args.widgets_profile.read_bytes(),
            prefer_security_cms_path=args.widgets_profile,
        )
        app_summary = validate_profile_dict(
            app_dict,
            expected_bundle_id=args.bundle_base_id,
            expected_team_id=args.team_id,
            expected_app_group=args.app_group,
            expected_cert_sha256=args.cert_sha256,
        )
        widgets_summary = validate_profile_dict(
            widgets_dict,
            expected_bundle_id=f"{args.bundle_base_id}.widgets",
            expected_team_id=args.team_id,
            expected_app_group=args.app_group,
            expected_cert_sha256=args.cert_sha256,
        )
        print(json.dumps({"verified": True, "app": app_summary, "widgets": widgets_summary}, indent=2))
        return 0

    if args.command == "with-keychain":
        child_cmd = list(args.child)
        if child_cmd and child_cmd[0] == "--":
            child_cmd = child_cmd[1:]
        if not child_cmd:
            raise ValueError("with-keychain requires a child command after --")
        if args.p12_password_file is not None:
            passphrase = args.p12_password_file.read_text(encoding="utf-8").strip()
        else:
            passphrase = os.environ.get(args.p12_password_env, "")
        if not passphrase:
            raise ValueError(f"Missing .p12 passphrase in env {args.p12_password_env} or --p12-password-file")
        return run_with_ephemeral_keychain(
            p12_path=args.p12,
            passphrase=passphrase,
            expected_cert_sha256=args.cert_sha256,
            child_command=child_cmd,
        )

    return 1


if __name__ == "__main__":
    sys.exit(main())
