#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLCHAIN_JSON="$ROOT_DIR/ci/ios-toolchain.json"
MODE="native"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      MODE="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

if [[ "$MODE" != "native" && "$MODE" != "release" ]]; then
  echo "Invalid --mode: $MODE (expected native or release)" >&2
  exit 1
fi

eval "$(python3 - "$TOOLCHAIN_JSON" <<'PY'
import json, shlex, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
print(f"DEFAULT_DEV_DIR={shlex.quote(data['xcode']['developerDir'])}")
print(f"DEPLOY_TARGET={shlex.quote(data['deploymentTarget'])}")
print(f"TARGET_DEVICE={shlex.quote(data['rust']['targets']['device'])}")
print(f"TARGET_SIM_INTEL={shlex.quote(data['rust']['targets']['simIntel'])}")
print(f"TARGET_SIM_ARM64={shlex.quote(data['rust']['targets']['simArm64'])}")
PY
)"

if [[ -z "${DEVELOPER_DIR:-}" && -d "$DEFAULT_DEV_DIR" ]]; then
  export DEVELOPER_DIR="$DEFAULT_DEV_DIR"
fi
export IPHONEOS_DEPLOYMENT_TARGET="$DEPLOY_TARGET"

XCFRAMEWORK_OUT="$ROOT_DIR/apps/ios/CoreFFI/Artifacts/TokenTrackerCoreFFI.xcframework"
SWIFT_SOURCES_OUT="$ROOT_DIR/apps/ios/CoreFFI/Sources/TokenTrackerCore"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "[build-xcframework] Building host tokentracker-ffi library and uniffi-bindgen..."
cargo build -p tokentracker-ffi --release
cargo build -p tokentracker-ffi --bin uniffi-bindgen --release

HOST_LIB=""
for candidate in \
  "$ROOT_DIR/target/release/libtokentracker_ffi.dylib" \
  "$ROOT_DIR/target/release/libtokentracker_ffi.so"; do
  if [[ -f "$candidate" ]]; then
    HOST_LIB="$candidate"
    break
  fi
done
if [[ -z "$HOST_LIB" ]]; then
  echo "Could not find compiled host dynamic library for tokentracker-ffi" >&2
  exit 1
fi

BINDGEN_OUT="$TMP_DIR/uniffi-swift"
"$ROOT_DIR/target/release/uniffi-bindgen" generate \
  --library "$HOST_LIB" \
  --language swift \
  --out-dir "$BINDGEN_OUT"

mkdir -p "$SWIFT_SOURCES_OUT"
cp "$BINDGEN_OUT/TokenTrackerCore.swift" "$SWIFT_SOURCES_OUT/TokenTrackerCore.swift"

HEADERS_DIR="$TMP_DIR/headers"
mkdir -p "$HEADERS_DIR"
cp "$BINDGEN_OUT/TokenTrackerCoreFFI.h" "$HEADERS_DIR/TokenTrackerCoreFFI.h"
cp "$BINDGEN_OUT/TokenTrackerCoreFFI.modulemap" "$HEADERS_DIR/module.modulemap"

echo "[build-xcframework] Building iOS Simulator static libraries ($TARGET_SIM_INTEL, $TARGET_SIM_ARM64)..."
SDKROOT="$(xcrun --sdk iphonesimulator --show-sdk-path)" \
  cargo build -p tokentracker-ffi --target "$TARGET_SIM_INTEL" --release
SDKROOT="$(xcrun --sdk iphonesimulator --show-sdk-path)" \
  cargo build -p tokentracker-ffi --target "$TARGET_SIM_ARM64" --release

SIM_FAT_LIB="$TMP_DIR/sim-fat/libtokentracker_ffi.a"
mkdir -p "$(dirname "$SIM_FAT_LIB")"
lipo -create \
  "$ROOT_DIR/target/$TARGET_SIM_INTEL/release/libtokentracker_ffi.a" \
  "$ROOT_DIR/target/$TARGET_SIM_ARM64/release/libtokentracker_ffi.a" \
  -output "$SIM_FAT_LIB"

XC_ARGS=(
  -library "$SIM_FAT_LIB"
  -headers "$HEADERS_DIR"
)

if [[ "$MODE" == "release" ]]; then
  echo "[build-xcframework] Building iOS Device static library ($TARGET_DEVICE)..."
  SDKROOT="$(xcrun --sdk iphoneos --show-sdk-path)" \
    cargo build -p tokentracker-ffi --target "$TARGET_DEVICE" --release
  XC_ARGS+=(
    -library "$ROOT_DIR/target/$TARGET_DEVICE/release/libtokentracker_ffi.a"
    -headers "$HEADERS_DIR"
  )
fi

rm -rf "$XCFRAMEWORK_OUT"
mkdir -p "$(dirname "$XCFRAMEWORK_OUT")"
xcodebuild -create-xcframework "${XC_ARGS[@]}" -output "$XCFRAMEWORK_OUT"

echo "[build-xcframework] Generated $XCFRAMEWORK_OUT and $SWIFT_SOURCES_OUT/TokenTrackerCore.swift (mode=$MODE)"
