#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLCHAIN_JSON="$ROOT_DIR/ci/ios-toolchain.json"
CANDIDATE_JSON="$ROOT_DIR/ci/testflight-candidate.json"
ARTIFACTS_DIR="$ROOT_DIR/artifacts/ios"
mkdir -p "$ARTIFACTS_DIR"
export PATH="$ROOT_DIR/.tools/bin:$ROOT_DIR/.tools/apple-venv/bin:$PATH"

PYTHON_BIN="$ROOT_DIR/.tools/apple-venv/bin/python3"
if [[ ! -x "$PYTHON_BIN" ]]; then
  PYTHON_BIN="$(command -v python3)"
fi

P12_PATH="${APPLE_P12_PATH:-}"
P12_PASSWORD_FILE="${APPLE_P12_PASSWORD_FILE:-}"
APP_PROFILE="${APPLE_APP_PROFILE_PATH:-}"
WIDGETS_PROFILE="${APPLE_WIDGETS_PROFILE_PATH:-}"
TEAM_ID="${APPLE_TEAM_ID:-}"
CERT_SHA256="${APPLE_CERT_SHA256:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --p12) P12_PATH="$2"; shift 2 ;;
    --p12-password-file) P12_PASSWORD_FILE="$2"; shift 2 ;;
    --app-profile) APP_PROFILE="$2"; shift 2 ;;
    --widgets-profile) WIDGETS_PROFILE="$2"; shift 2 ;;
    --team-id) TEAM_ID="$2"; shift 2 ;;
    --cert-sha256) CERT_SHA256="$2"; shift 2 ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

eval "$(python3 - "$TOOLCHAIN_JSON" "$CANDIDATE_JSON" <<'PY'
import json, shlex, sys
tc = json.load(open(sys.argv[1], encoding="utf-8"))
cand = json.load(open(sys.argv[2], encoding="utf-8"))
print(f"DEFAULT_DEV_DIR={shlex.quote(tc['xcode']['developerDir'])}")
print(f"BUNDLE_BASE_ID={shlex.quote(cand['bundleBaseId'])}")
print(f"WIDGETS_BUNDLE_ID={shlex.quote(cand['widgetsBundleId'])}")
print(f"APP_GROUP_ID={shlex.quote(cand['appGroupId'])}")
print(f"MARKETING_VERSION={shlex.quote(str(cand['marketingVersion']))}")
print(f"BUILD_NUMBER={shlex.quote(str(cand['buildNumber']))}")
print(f"CANDIDATE_TEAM_ID={shlex.quote(str(cand['teamId']))}")
print(f"CANDIDATE_CERT_SHA={shlex.quote(str(cand['certificateSha256']))}")
PY
)"

TEAM_ID="${TEAM_ID:-$CANDIDATE_TEAM_ID}"
CERT_SHA256="${CERT_SHA256:-$CANDIDATE_CERT_SHA}"

if [[ -z "$P12_PATH" || -z "$APP_PROFILE" || -z "$WIDGETS_PROFILE" ]]; then
  echo "archive-ios.sh requires --p12, --app-profile, and --widgets-profile (or env vars)" >&2
  exit 1
fi

if [[ -z "${DEVELOPER_DIR:-}" && -d "$DEFAULT_DEV_DIR" ]]; then
  export DEVELOPER_DIR="$DEFAULT_DEV_DIR"
fi

echo "[archive-ios] Validating App Store Distribution provisioning profiles..."
PROFILES_JSON="$("$PYTHON_BIN" "$ROOT_DIR/scripts/ios/azure-archive.py" validate-profiles \
  --app-profile "$APP_PROFILE" \
  --widgets-profile "$WIDGETS_PROFILE" \
  --bundle-base-id "$BUNDLE_BASE_ID" \
  --app-group "$APP_GROUP_ID" \
  --team-id "$TEAM_ID" \
  --cert-sha256 "$CERT_SHA256")"
echo "$PROFILES_JSON"

echo "[archive-ios] Regenerating Xcode project..."
(cd "$ROOT_DIR/apps/ios" && xcodegen generate --spec project.yml)

ARCHIVE_PATH="$ARTIFACTS_DIR/TokenTracker.xcarchive"
EXPORT_DIR="$ARTIFACTS_DIR/export"
EXPORT_PLIST="$ARTIFACTS_DIR/ExportOptions.plist"
rm -rf "$ARCHIVE_PATH" "$EXPORT_DIR"
mkdir -p "$EXPORT_DIR"

"$PYTHON_BIN" - "$PROFILES_JSON" "$TEAM_ID" "$BUNDLE_BASE_ID" "$WIDGETS_BUNDLE_ID" "$EXPORT_PLIST" <<'PY'
import json, plistlib, sys
summary = json.loads(sys.argv[1])
team_id = sys.argv[2]
bundle_base_id = sys.argv[3]
widgets_bundle_id = sys.argv[4]
out_path = sys.argv[5]

payload = {
    "method": "app-store-connect",
    "signingStyle": "manual",
    "teamID": team_id,
    "manageAppVersionAndBuildNumber": False,
    "uploadSymbols": True,
    "stripSwiftSymbols": True,
    "provisioningProfiles": {
        bundle_base_id: summary["app"]["name"],
        widgets_bundle_id: summary["widgets"]["name"],
    },
}
with open(out_path, "wb") as fh:
    plistlib.dump(payload, fh)
PY

# Xcode 26 discovers profiles by UUID in the per-user Xcode directory.
# Both profiles were decoded and identity-checked above, on an ephemeral agent.
PROFILE_DIR="$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles"
mkdir -p "$PROFILE_DIR"
APP_PROFILE_UUID="$(node -p 'JSON.parse(process.argv[1]).app.uuid' "$PROFILES_JSON")"
WIDGETS_PROFILE_UUID="$(node -p 'JSON.parse(process.argv[1]).widgets.uuid' "$PROFILES_JSON")"
for uuid in "$APP_PROFILE_UUID" "$WIDGETS_PROFILE_UUID"; do
  [[ "$uuid" =~ ^[0-9A-Fa-f]{8}(-[0-9A-Fa-f]{4}){3}-[0-9A-Fa-f]{12}$ ]] || exit 1
done
install -m 600 "$APP_PROFILE" "$PROFILE_DIR/$APP_PROFILE_UUID.mobileprovision"
install -m 600 "$WIDGETS_PROFILE" "$PROFILE_DIR/$WIDGETS_PROFILE_UUID.mobileprovision"

KEYCHAIN_AUTH_ARGS=(--p12-password-env APPLE_P12_PASSWORD)
if [[ -n "$P12_PASSWORD_FILE" ]]; then
  KEYCHAIN_AUTH_ARGS=(--p12-password-file "$P12_PASSWORD_FILE")
fi

echo "[archive-ios] Applying manual profiles & archiving inside ephemeral keychain..."
"$PYTHON_BIN" "$ROOT_DIR/scripts/ios/azure-archive.py" with-keychain \
  --p12 "$P12_PATH" \
  "${KEYCHAIN_AUTH_ARGS[@]}" \
  --cert-sha256 "$CERT_SHA256" \
  -- bash -c '
    set -euo pipefail
    xcodebuild archive \
      -project "$1" \
      -scheme TokenTracker \
      -configuration Release \
      -destination "generic/platform=iOS" \
      -archivePath "$4" \
      DEVELOPMENT_TEAM="$5" \
      CODE_SIGN_STYLE=Manual \
      CODE_SIGN_IDENTITY="Apple Distribution" \
      TT_APP_PROFILE="$2" TT_WIDGETS_PROFILE="$3" \
      CURRENT_PROJECT_VERSION="$6" \
      MARKETING_VERSION="$7"
    xcodebuild -exportArchive \
      -archivePath "$4" \
      -exportPath "$8" \
      -exportOptionsPlist "$9"
  ' _ \
    "$ROOT_DIR/apps/ios/TokenTracker.xcodeproj" \
    "$APP_PROFILE_UUID" \
    "$WIDGETS_PROFILE_UUID" \
    "$ARCHIVE_PATH" \
    "$TEAM_ID" \
    "$BUILD_NUMBER" \
    "$MARKETING_VERSION" \
    "$EXPORT_DIR" \
    "$EXPORT_PLIST"

IPA_FILE="$(find "$EXPORT_DIR" -maxdepth 1 -type f -name '*.ipa' | head -n 1)"
if [[ -z "$IPA_FILE" ]]; then
  echo "xcodebuild -exportArchive did not produce an .ipa file in $EXPORT_DIR" >&2
  exit 1
fi
cp "$IPA_FILE" "$ARTIFACTS_DIR/TokenTracker.ipa"

"$PYTHON_BIN" "$ROOT_DIR/scripts/ios/verify-device-build.py" \
  --xcarchive "$ARCHIVE_PATH" \
  --ipa "$ARTIFACTS_DIR/TokenTracker.ipa" \
  --candidate "$CANDIDATE_JSON" \
  --team-id "$TEAM_ID" \
  --cert-sha256 "$CERT_SHA256" \
  --output "$ARTIFACTS_DIR/device-signing.json"

RELEASE_PKG_DIR="$ARTIFACTS_DIR/release-package"
rm -rf "$RELEASE_PKG_DIR"
mkdir -p "$RELEASE_PKG_DIR"
cp "$ARTIFACTS_DIR/TokenTracker.ipa" "$RELEASE_PKG_DIR/TokenTracker.ipa"
cp "$ARTIFACTS_DIR/device-signing.json" "$RELEASE_PKG_DIR/device-signing.json"
cp "$EXPORT_PLIST" "$RELEASE_PKG_DIR/ExportOptions.plist"
if [[ -d "$ARCHIVE_PATH/dSYMs" ]]; then
  cp -R "$ARCHIVE_PATH/dSYMs" "$RELEASE_PKG_DIR/dSYMs"
fi
