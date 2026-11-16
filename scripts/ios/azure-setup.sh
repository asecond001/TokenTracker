#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLCHAIN_JSON="$ROOT_DIR/ci/ios-toolchain.json"
WITH_APPLE_TOOLS=0
WHEELHOUSE_DIR="${WHEELHOUSE_DIR:-$ROOT_DIR/artifacts/ios/wheelhouse}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --with-apple-tools)
      WITH_APPLE_TOOLS=1
      shift
      ;;
    --wheelhouse-dir)
      WHEELHOUSE_DIR="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

eval "$(python3 - "$TOOLCHAIN_JSON" <<'PY'
import json, shlex, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
print(f"EXPECTED_DEV_DIR={shlex.quote(data['xcode']['developerDir'])}")
print(f"EXPECTED_XCODE_VER={shlex.quote(data['xcode']['version'])}")
print(f"EXPECTED_SDK={shlex.quote(data['iosSdk'])}")
print(f"XCODEGEN_VER={shlex.quote(data['xcodegen']['version'])}")
print(f"XCODEGEN_URL={shlex.quote(data['xcodegen']['downloadUrl'])}")
print(f"NODE_VER={shlex.quote(data['nodeVersion'])}")
print(f"RUST_TARGET_DEVICE={shlex.quote(data['rust']['targets']['device'])}")
print(f"RUST_TARGET_SIM_INTEL={shlex.quote(data['rust']['targets']['simIntel'])}")
print(f"RUST_TARGET_SIM_ARM64={shlex.quote(data['rust']['targets']['simArm64'])}")
PY
)"

if [[ -d "$EXPECTED_DEV_DIR" ]]; then
  if command -v sudo >/dev/null 2>&1; then
    sudo xcode-select -s "$EXPECTED_DEV_DIR"
  else
    export DEVELOPER_DIR="$EXPECTED_DEV_DIR"
  fi
  export DEVELOPER_DIR="$EXPECTED_DEV_DIR"
else
  echo "Expected Xcode developer directory not found: $EXPECTED_DEV_DIR" >&2
  exit 1
fi

echo "[azure-setup] Xcode version:"
xcodebuild -version
SDK_VER="$(xcrun --sdk iphoneos --show-sdk-version)"
echo "[azure-setup] iOS SDK version: $SDK_VER (expected $EXPECTED_SDK)"

echo "[azure-setup] Node version: $(node -v) (toolchain target $NODE_VER)"

LOCAL_BIN="$ROOT_DIR/.tools/bin"
mkdir -p "$LOCAL_BIN"
export PATH="$LOCAL_BIN:$PATH"

XCODEGEN_OK=0
if command -v xcodegen >/dev/null 2>&1; then
  CURRENT_XG="$(xcodegen --version || true)"
  if [[ "$CURRENT_XG" == *"$XCODEGEN_VER"* ]]; then
    XCODEGEN_OK=1
  fi
fi

if [[ "$XCODEGEN_OK" -eq 0 ]]; then
  echo "[azure-setup] Installing pinned XcodeGen $XCODEGEN_VER"
  TMP_XG="$(mktemp -d)"
  curl -fsSL "$XCODEGEN_URL" -o "$TMP_XG/xcodegen.zip"
  unzip -q "$TMP_XG/xcodegen.zip" -d "$TMP_XG/unzipped"
  XG_BIN="$(find "$TMP_XG/unzipped" -type f -name xcodegen | head -n 1)"
  if [[ -z "$XG_BIN" ]]; then
    echo "Failed to locate xcodegen binary inside downloaded archive" >&2
    rm -rf "$TMP_XG"
    exit 1
  fi
  cp "$XG_BIN" "$LOCAL_BIN/xcodegen"
  chmod +x "$LOCAL_BIN/xcodegen"
  rm -rf "$TMP_XG"
fi
echo "[azure-setup] XcodeGen: $("$LOCAL_BIN/xcodegen" --version 2>/dev/null || xcodegen --version)"

rustup target add "$RUST_TARGET_DEVICE" "$RUST_TARGET_SIM_INTEL" "$RUST_TARGET_SIM_ARM64"
cargo --version

if [[ "$WITH_APPLE_TOOLS" -eq 1 ]]; then
  REQ_FILE="$ROOT_DIR/ci/apple-tools-requirements.txt"
  VENV_DIR="$ROOT_DIR/.tools/apple-venv"
  if [[ ! -x "$VENV_DIR/bin/python3" ]]; then
    python3 -m venv "$VENV_DIR"
  fi
  if [[ -f "$WHEELHOUSE_DIR/manifest.json" ]]; then
    echo "[azure-setup] Verifying cached Python wheelhouse at $WHEELHOUSE_DIR"
    "$VENV_DIR/bin/python3" "$ROOT_DIR/scripts/ios/apple-wheelhouse.py" verify \
      --requirements "$REQ_FILE" \
      --wheelhouse-dir "$WHEELHOUSE_DIR"
  else
    echo "[azure-setup] Building Python wheelhouse into $WHEELHOUSE_DIR"
    "$VENV_DIR/bin/python3" "$ROOT_DIR/scripts/ios/apple-wheelhouse.py" build \
      --requirements "$REQ_FILE" \
      --wheelhouse-dir "$WHEELHOUSE_DIR"
  fi
  "$VENV_DIR/bin/python3" "$ROOT_DIR/scripts/ios/apple-wheelhouse.py" install \
    --requirements "$REQ_FILE" \
    --wheelhouse-dir "$WHEELHOUSE_DIR" \
    --python-exe "$VENV_DIR/bin/python3"
fi
