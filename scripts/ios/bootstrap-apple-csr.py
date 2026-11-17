#!/usr/bin/env python3
"""Offline Apple Distribution CSR generator and local .p12 packer (Windows/macOS)."""

from __future__ import annotations

import argparse
import json
import os
import secrets
import sys
from pathlib import Path
from typing import Any

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.serialization import pkcs12
from cryptography.x509.oid import NameOID


def cert_der_sha256(cert: x509.Certificate) -> str:
    return cert.fingerprint(hashes.SHA256()).hex().lower()


def load_certificate_auto(raw: bytes) -> x509.Certificate:
    trimmed = raw.lstrip()
    if trimmed.startswith(b"-----BEGIN"):
        return x509.load_pem_x509_certificate(raw)
    try:
        return x509.load_der_x509_certificate(raw)
    except ValueError:
        return x509.load_pem_x509_certificate(raw)


def _restrict_file_permissions(path: Path) -> None:
    if os.name == "posix":
        os.chmod(path, 0o600)


def generate_csr(
    private_key_out: Path,
    csr_out: Path,
    common_name: str = "TokenVane Apple Distribution",
    organization: str = "TokenVane",
    country: str = "US",
) -> dict[str, Any]:
    if len(country) != 2:
        raise ValueError("country must be a 2-letter ISO code")

    private_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    csr = (
        x509.CertificateSigningRequestBuilder()
        .subject_name(
            x509.Name(
                [
                    x509.NameAttribute(NameOID.COUNTRY_NAME, country),
                    x509.NameAttribute(NameOID.ORGANIZATION_NAME, organization),
                    x509.NameAttribute(NameOID.COMMON_NAME, common_name),
                ]
            )
        )
        .sign(private_key, hashes.SHA256())
    )

    private_key_out.parent.mkdir(parents=True, exist_ok=True)
    csr_out.parent.mkdir(parents=True, exist_ok=True)

    private_key_out.write_bytes(
        private_key.private_bytes(
            encoding=serialization.Encoding.PEM,
            format=serialization.PrivateFormat.PKCS8,
            encryption_algorithm=serialization.NoEncryption(),
        )
    )
    _restrict_file_permissions(private_key_out)

    csr_out.write_bytes(csr.public_bytes(serialization.Encoding.PEM))

    pub_der = private_key.public_key().public_bytes(
        encoding=serialization.Encoding.DER,
        format=serialization.PublicFormat.SubjectPublicKeyInfo,
    )
    digest = hashes.Hash(hashes.SHA256())
    digest.update(pub_der)
    pub_sha256 = digest.finalize().hex().lower()

    return {
        "privateKeyPath": str(private_key_out),
        "csrPath": str(csr_out),
        "commonName": common_name,
        "publicKeySha256": pub_sha256,
    }


def _select_pkcs12_encryption(password_bytes: bytes, legacy_pbes1: bool) -> Any:
    if legacy_pbes1 and hasattr(pkcs12, "PBES"):
        return (
            serialization.PrivateFormat.PKCS12.encryption_builder()
            .kdf_rounds(2048)
            .key_cert_algorithm(pkcs12.PBES.PBESv1SHA1And3KeyTripleDESCBC)
            .hmac_hash(hashes.SHA1())
            .build(password_bytes)
        )
    return serialization.BestAvailableEncryption(password_bytes)


def pack_p12(
    private_key_path: Path,
    cer_path: Path,
    p12_out: Path,
    password_out: Path | None = None,
    friendly_name: str = "TokenVane Distribution",
    password: str | None = None,
    legacy_pbes1: bool = False,
) -> dict[str, Any]:
    key = serialization.load_pem_private_key(private_key_path.read_bytes(), password=None)
    if not isinstance(key, rsa.RSAPrivateKey):
        raise ValueError("Expected RSA private key in PEM file")

    cert = load_certificate_auto(cer_path.read_bytes())
    cert_pub = cert.public_key()
    if not isinstance(cert_pub, rsa.RSAPublicKey):
        raise ValueError("Certificate public key is not RSA")
    if key.public_key().public_numbers() != cert_pub.public_numbers():
        raise ValueError("Private key does not match certificate public key")

    resolved_password = password if password is not None else secrets.token_hex(32)
    if not resolved_password:
        raise ValueError("P12 password must not be empty")

    encryption = _select_pkcs12_encryption(resolved_password.encode("utf-8"), legacy_pbes1)
    p12_bytes = pkcs12.serialize_key_and_certificates(
        name=friendly_name.encode("utf-8"),
        key=key,
        cert=cert,
        cas=None,
        encryption_algorithm=encryption,
    )

    p12_out.parent.mkdir(parents=True, exist_ok=True)
    p12_out.write_bytes(p12_bytes)
    _restrict_file_permissions(p12_out)

    resolved_pw_path: str | None = None
    if password_out is not None:
        password_out.parent.mkdir(parents=True, exist_ok=True)
        password_out.write_text(resolved_password, encoding="utf-8")
        _restrict_file_permissions(password_out)
        resolved_pw_path = str(password_out)

    not_after = getattr(cert, "not_valid_after_utc", None) or cert.not_valid_after
    return {
        "p12Path": str(p12_out),
        "passwordPath": resolved_pw_path,
        "certificateSha256": cert_der_sha256(cert),
        "subject": cert.subject.rfc4514_string(),
        "notValidAfter": not_after.isoformat(),
        "legacyPbes1": legacy_pbes1,
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Offline Apple CSR generator and .p12 packer")
    sub = parser.add_subparsers(dest="command", required=True)

    csr_cmd = sub.add_parser("generate-csr", help="Generate RSA-2048 private key and .certSigningRequest")
    csr_cmd.add_argument(
        "--key-out",
        type=Path,
        default=Path("artifacts/apple-signing/tokenvane_distribution.key"),
    )
    csr_cmd.add_argument(
        "--csr-out",
        type=Path,
        default=Path("artifacts/apple-signing/tokenvane_distribution.certSigningRequest"),
    )
    csr_cmd.add_argument("--common-name", default="TokenVane Apple Distribution")
    csr_cmd.add_argument("--org", default="TokenVane")
    csr_cmd.add_argument("--country", default="US")

    p12_cmd = sub.add_parser("pack-p12", help="Combine private key and Apple .cer into an encrypted .p12")
    p12_cmd.add_argument(
        "--key-in",
        type=Path,
        default=Path("artifacts/apple-signing/tokenvane_distribution.key"),
    )
    p12_cmd.add_argument("--cer", "--cer-in", dest="cer_in", required=True, type=Path)
    p12_cmd.add_argument(
        "--p12-out",
        type=Path,
        default=Path("artifacts/apple-signing/tokenvane_distribution.p12"),
    )
    p12_cmd.add_argument(
        "--password-out",
        type=Path,
        default=Path("artifacts/apple-signing/tokenvane_p12_password.txt"),
    )
    p12_cmd.add_argument("--friendly-name", default="TokenVane Distribution")
    p12_cmd.add_argument("--legacy-pbes1", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.command == "generate-csr":
        result = generate_csr(
            private_key_out=args.key_out,
            csr_out=args.csr_out,
            common_name=args.common_name,
            organization=args.org,
            country=args.country,
        )
        print(json.dumps(result, indent=2))
        return 0

    if args.command == "pack-p12":
        pw_out = args.password_out
        if pw_out is None:
            pw_out = args.p12_out.with_suffix(".password.txt")
        result = pack_p12(
            private_key_path=args.key_in,
            cer_path=args.cer_in,
            p12_out=args.p12_out,
            password_out=pw_out,
            friendly_name=args.friendly_name,
            legacy_pbes1=bool(args.legacy_pbes1),
        )
        print(json.dumps(result, indent=2))
        return 0

    return 1


if __name__ == "__main__":
    sys.exit(main())
