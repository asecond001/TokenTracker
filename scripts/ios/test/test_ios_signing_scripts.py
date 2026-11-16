#!/usr/bin/env python3
"""Offline unit tests for iOS CSR, .p12, wheelhouse, provisioning profile, and IPA verifier scripts."""

from __future__ import annotations

import datetime as dt
import hashlib
import importlib.util
import plistlib
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path
from types import SimpleNamespace

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.x509.oid import NameOID

SCRIPTS_DIR = Path(__file__).resolve().parents[1]


def load_script_module(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, str(SCRIPTS_DIR / filename))
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


bootstrap_mod = load_script_module("bootstrap_apple_csr", "bootstrap-apple-csr.py")
wheelhouse_mod = load_script_module("apple_wheelhouse", "apple-wheelhouse.py")
archive_mod = load_script_module("azure_archive", "azure-archive.py")
verify_mod = load_script_module("verify_device_build", "verify-device-build.py")


def build_self_signed_cert_from_csr(csr_pem: bytes, key_pem: bytes) -> bytes:
    csr = x509.load_pem_x509_csr(csr_pem)
    key = serialization.load_pem_private_key(key_pem, password=None)
    now = dt.datetime.now(dt.timezone.utc)
    cert = (
        x509.CertificateBuilder()
        .subject_name(csr.subject)
        .issuer_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Apple Worldwide Developer Relations")]))
        .public_key(csr.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - dt.timedelta(days=1))
        .not_valid_after(now + dt.timedelta(days=365))
        .sign(key, hashes.SHA256())
    )
    return cert.public_bytes(serialization.Encoding.DER)


class BootstrapAndArchiveCryptoTests(unittest.TestCase):
    def test_csr_p12_roundtrip_and_legacy_reencode(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            key_path = root / "dist.key.pem"
            csr_path = root / "dist.certSigningRequest"
            cer_path = root / "distribution.cer"
            p12_path = root / "distribution.p12"
            pw_path = root / "distribution.password.txt"

            bootstrap_mod.generate_csr(key_path, csr_path)
            cer_der = build_self_signed_cert_from_csr(csr_path.read_bytes(), key_path.read_bytes())
            cer_path.write_bytes(cer_der)
            expected_sha = hashlib.sha256(cer_der).hexdigest().lower()

            pack_res = bootstrap_mod.pack_p12(key_path, cer_path, p12_path, pw_path, legacy_pbes1=True)
            self.assertEqual(pack_res["certificateSha256"], expected_sha)
            password = pw_path.read_text(encoding="utf-8")
            self.assertEqual(len(password), 64)

            legacy_out = root / "reencoded.p12"
            one_time_pw, re_sha = archive_mod.reencode_p12_legacy(
                p12_path.read_bytes(), password, expected_sha, legacy_out
            )
            self.assertEqual(re_sha, expected_sha)
            key, cert, _ = pkcs12.load_key_and_certificates(legacy_out.read_bytes(), one_time_pw.encode("utf-8"))
            self.assertIsNotNone(key)
            self.assertIsNotNone(cert)

            with self.assertRaises(ValueError):
                archive_mod.reencode_p12_legacy(p12_path.read_bytes(), password, "0" * 64, legacy_out)

    def test_wheelhouse_manifest_detects_tamper_and_extra_wheel(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            req = root / "reqs.txt"
            req.write_text("cryptography==50.0.1\n", encoding="utf-8")
            wh_dir = root / "wheelhouse"
            wh_dir.mkdir()
            whl = wh_dir / "cryptography-50.0.1-cp312-cp312-macosx_14_0_x86_64.whl"
            whl.write_bytes(b"fake-wheel-bytes")

            wheelhouse_mod.write_manifest(wh_dir, req)
            ok = wheelhouse_mod.verify_wheelhouse(wh_dir, req)
            self.assertTrue(ok["verified"])

            whl.write_bytes(b"tampered-wheel-bytes")
            with self.assertRaises(ValueError):
                wheelhouse_mod.verify_wheelhouse(wh_dir, req)

            whl.write_bytes(b"fake-wheel-bytes")
            (wh_dir / "rogue-1.0.0-py3-none-any.whl").write_bytes(b"rogue")
            with self.assertRaises(ValueError):
                wheelhouse_mod.verify_wheelhouse(wh_dir, req)

    def test_profile_validator_accepts_distribution_and_rejects_invalid(self) -> None:
        cert_der = b"dummy-cert-der-bytes"
        cert_sha = hashlib.sha256(cert_der).hexdigest().lower()
        now = dt.datetime.now(dt.timezone.utc)
        valid_profile = {
            "Name": "TokenTracker AppStore",
            "UUID": "11111111-2222-3333-4444-555555555555",
            "TeamIdentifier": ["TEAM123456"],
            "ExpirationDate": now + dt.timedelta(days=30),
            "DeveloperCertificates": [cert_der],
            "Entitlements": {
                "get-task-allow": False,
                "application-identifier": "TEAM123456.com.tokentracker.app",
                "com.apple.security.application-groups": ["group.com.tokentracker.app"],
            },
        }
        wrapped_cms = b"CMS-HEADER" + plistlib.dumps(valid_profile) + b"CMS-TRAILER"
        decoded = archive_mod.decode_mobileprovision_bytes(wrapped_cms)
        res = archive_mod.validate_profile_dict(
            decoded,
            expected_bundle_id="com.tokentracker.app",
            expected_team_id="TEAM123456",
            expected_app_group="group.com.tokentracker.app",
            expected_cert_sha256=cert_sha,
            now_utc=now,
        )
        self.assertEqual(res["teamId"], "TEAM123456")

        for mutator in [
            lambda d: d.update({"ExpirationDate": now - dt.timedelta(days=1)}),
            lambda d: d.update({"ProvisionedDevices": ["udid-1"]}),
            lambda d: d.update({"TeamIdentifier": ["OTHER999"]}),
            lambda d: d["Entitlements"].update({"get-task-allow": True}),
            lambda d: d["Entitlements"].update({"com.apple.security.application-groups": ["group.other"]}),
            lambda d: d.update({"DeveloperCertificates": [b"wrong-cert"]}),
        ]:
            broken = plistlib.loads(plistlib.dumps(valid_profile))
            mutator(broken)
            with self.assertRaises(ValueError):
                archive_mod.validate_profile_dict(
                    broken,
                    expected_bundle_id="com.tokentracker.app",
                    expected_team_id="TEAM123456",
                    expected_app_group="group.com.tokentracker.app",
                    expected_cert_sha256=cert_sha,
                    now_utc=now,
                )

    def test_ephemeral_keychain_cleans_up_temp_files_on_failure(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            key_path = root / "k.pem"
            csr_path = root / "k.csr"
            cer_path = root / "k.cer"
            p12_path = root / "k.p12"
            pw_path = root / "k.pass"
            bootstrap_mod.generate_csr(key_path, csr_path)
            cer_der = build_self_signed_cert_from_csr(csr_path.read_bytes(), key_path.read_bytes())
            cer_path.write_bytes(cer_der)
            bootstrap_mod.pack_p12(key_path, cer_path, p12_path, pw_path)
            cert_sha = hashlib.sha256(cer_der).hexdigest().lower()

            calls: list[list[str]] = []

            def fake_runner(cmd: list[str], *, check: bool = True, capture_output: bool = False, env=None):
                calls.append(cmd)
                if cmd == ["failing-child-cmd"]:
                    raise subprocess.CalledProcessError(returncode=2, cmd=cmd)
                if cmd[:3] == ["security", "default-keychain", "-d"] and len(cmd) == 4:
                    return SimpleNamespace(returncode=0, stdout='"/Users/runner/Library/Keychains/login.keychain-db"\n', stderr="")
                if cmd[:3] == ["security", "list-keychains", "-d"] and len(cmd) == 4:
                    return SimpleNamespace(returncode=0, stdout='"/Users/runner/Library/Keychains/login.keychain-db"\n', stderr="")
                return SimpleNamespace(returncode=0, stdout="", stderr="")

            kc_root = root / "agent-tmp"
            kc_root.mkdir()
            with self.assertRaises(subprocess.CalledProcessError):
                archive_mod.run_with_ephemeral_keychain(
                    p12_path=p12_path,
                    passphrase=pw_path.read_text(encoding="utf-8"),
                    expected_cert_sha256=cert_sha,
                    child_command=["failing-child-cmd"],
                    temp_root=kc_root,
                    runner=fake_runner,
                    install_signal_handlers=False,
                )

            self.assertEqual(list(kc_root.iterdir()), [])
            self.assertTrue(any(c[:2] == ["security", "delete-keychain"] for c in calls))
            self.assertTrue(
                any(
                    c[:4] == ["security", "default-keychain", "-d", "user"]
                    and len(c) == 6
                    and "login.keychain-db" in c[5]
                    for c in calls
                )
            )

    def test_verify_device_build_verifies_app_and_widget_bundles_and_sha256(self) -> None:
        cert_der = b"simulated-signing-cert-der"
        cert_sha = hashlib.sha256(cert_der).hexdigest().lower()
        team_id = "TEAM123456"

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            xcarchive = root / "TokenTracker.xcarchive"
            app_dir = xcarchive / "Products" / "Applications" / "TokenTracker.app"
            widgets_dir = app_dir / "PlugIns" / "TokenTrackerWidgets.appex"
            app_dir.mkdir(parents=True)
            widgets_dir.mkdir(parents=True)

            with (app_dir / "Info.plist").open("wb") as fh:
                plistlib.dump(
                    {
                        "CFBundleIdentifier": "com.tokentracker.app",
                        "CFBundleShortVersionString": "0.3.0",
                        "CFBundleVersion": "12",
                    },
                    fh,
                )
            with (widgets_dir / "Info.plist").open("wb") as fh:
                plistlib.dump(
                    {
                        "CFBundleIdentifier": "com.tokentracker.app.widgets",
                        "CFBundleShortVersionString": "0.3.0",
                        "CFBundleVersion": "12",
                    },
                    fh,
                )

            ipa_path = root / "TokenTracker.ipa"
            with zipfile.ZipFile(ipa_path, "w") as zf:
                zf.write(app_dir / "Info.plist", "Payload/TokenTracker.app/Info.plist")
                zf.write(widgets_dir / "Info.plist", "Payload/TokenTracker.app/PlugIns/TokenTrackerWidgets.appex/Info.plist")

            def fake_codesign(cmd: list[str], *, check: bool = True, capture_output: bool = True):
                target = cmd[-1]
                bundle_id = "com.tokentracker.app.widgets" if target.endswith(".appex") else "com.tokentracker.app"
                if "--verify" in cmd:
                    return SimpleNamespace(returncode=0, stdout="", stderr=f"{target}: valid on disk")
                if "-dvv" in cmd:
                    return SimpleNamespace(returncode=0, stdout="", stderr=f"TeamIdentifier={team_id}\nAuthority=Apple Distribution")
                for arg in cmd:
                    if arg.startswith("--extract-certificates="):
                        prefix = arg.split("=", 1)[1]
                        Path(f"{prefix}0").write_bytes(cert_der)
                        return SimpleNamespace(returncode=0, stdout="", stderr="")
                if "--entitlements" in cmd:
                    ent_bytes = plistlib.dumps(
                        {
                            "get-task-allow": False,
                            "application-identifier": f"{team_id}.{bundle_id}",
                            "com.apple.security.application-groups": ["group.com.tokentracker.app"],
                        }
                    ).decode("utf-8")
                    return SimpleNamespace(returncode=0, stdout=ent_bytes, stderr="")
                raise AssertionError(f"Unexpected command: {cmd}")

            report = verify_mod.verify_signing_artifacts(
                xcarchive_path=xcarchive,
                ipa_path=ipa_path,
                bundle_base_id="com.tokentracker.app",
                widgets_bundle_id="com.tokentracker.app.widgets",
                app_group_id="group.com.tokentracker.app",
                team_id=team_id,
                certificate_sha256=cert_sha,
                marketing_version="0.3.0",
                build_number="12",
                runner=fake_codesign,
            )
            self.assertTrue(report["verified"])
            self.assertEqual(report["bundles"]["app"]["bundleId"], "com.tokentracker.app")
            self.assertEqual(report["bundles"]["widgets"]["bundleId"], "com.tokentracker.app.widgets")
            self.assertEqual(len(report["ipa"]["sha256"]), 64)


if __name__ == "__main__":
    unittest.main()
