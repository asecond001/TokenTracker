#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLCHAIN_JSON="$ROOT_DIR/ci/ios-toolchain.json"
ARTIFACTS_DIR="$ROOT_DIR/artifacts/ios"
mkdir -p "$ARTIFACTS_DIR"
export PATH="$ROOT_DIR/.tools/bin:$PATH"

eval "$(python3 - "$TOOLCHAIN_JSON" <<'PY'
import json, shlex, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
print(f"DEFAULT_DEV_DIR={shlex.quote(data['xcode']['developerDir'])}")
print(f"PREFERRED_SIMS_JSON={shlex.quote(json.dumps(data['simulator']['preferredNames']))}")
print(f"RUNTIME_PREFIX={shlex.quote(data['simulator']['runtimePrefix'])}")
PY
)"

if [[ -z "${DEVELOPER_DIR:-}" && -d "$DEFAULT_DEV_DIR" ]]; then
  export DEVELOPER_DIR="$DEFAULT_DEV_DIR"
fi

echo "[native-check] Generating Xcode project via XcodeGen..."
(cd "$ROOT_DIR/apps/ios" && xcodegen generate --spec project.yml)

xcrun simctl list devices available -j > "$ARTIFACTS_DIR/simulators.json"
SIM_UDID="$(python3 - "$PREFERRED_SIMS_JSON" "$RUNTIME_PREFIX" "$ARTIFACTS_DIR/simulators.json" <<'PY'
import json, sys
preferred = json.loads(sys.argv[1])
runtime_prefix = sys.argv[2]
with open(sys.argv[3], encoding="utf-8") as devices_file:
    data = json.load(devices_file)
devices_by_rt = data.get("devices", {})

candidates = []
for rt, devs in devices_by_rt.items():
    if runtime_prefix and not rt.startswith(runtime_prefix):
        continue
    for d in devs:
        if d.get("isAvailable") and d.get("name", "").startswith("iPhone"):
            candidates.append((rt, d))

if not candidates:
    for rt, devs in devices_by_rt.items():
        if "iOS-" in rt:
            for d in devs:
                if d.get("isAvailable") and d.get("name", "").startswith("iPhone"):
                    candidates.append((rt, d))

for name in preferred:
    for _rt, d in candidates:
        if d.get("name") == name:
            print(d["udid"])
            sys.exit(0)

if candidates:
    print(candidates[0][1]["udid"])
    sys.exit(0)

sys.exit(1)
PY
)"

if [[ -z "$SIM_UDID" ]]; then
  echo "No available iOS Simulator found matching $RUNTIME_PREFIX" >&2
  exit 1
fi
echo "[native-check] Selected Simulator UDID: $SIM_UDID"
xcrun simctl boot "$SIM_UDID" >/dev/null 2>&1 || true

RESULT_BUNDLE="$ARTIFACTS_DIR/TokenTrackerTests.xcresult"
rm -rf "$RESULT_BUNDLE"

xcodebuild test \
  -project "$ROOT_DIR/apps/ios/TokenTracker.xcodeproj" \
  -scheme TokenTracker \
  -destination "platform=iOS Simulator,id=$SIM_UDID" \
  -derivedDataPath "$ARTIFACTS_DIR/DerivedData" \
  -resultBundlePath "$RESULT_BUNDLE" \
  COMPILER_INDEX_STORE_ENABLE=NO

xcrun xcresulttool get test-results summary --path "$RESULT_BUNDLE" --format json \
  > "$ARTIFACTS_DIR/raw-xcresult-summary.json"

python3 - "$ARTIFACTS_DIR/raw-xcresult-summary.json" "$ARTIFACTS_DIR/test-summary.json" <<'PY'
import json, sys
raw = json.load(open(sys.argv[1], encoding="utf-8"))
passed = int(raw.get("passedTests", 0))
failed = int(raw.get("failedTests", 0))
skipped = int(raw.get("skippedTests", 0))
executed = int(raw.get("totalTestCount", passed + failed + skipped))
summary = {
    "testsExecuted": executed,
    "testsPassed": passed,
    "testsFailed": failed,
    "testsSkipped": skipped,
    "allowedSkips": [],
    "result": raw.get("result", "Unknown")
}
with open(sys.argv[2], "w", encoding="utf-8") as fh:
    json.dump(summary, fh, indent=2)
PY

node "$ROOT_DIR/scripts/ios/verify-native-test-result.mjs" \
  --summary "$ARTIFACTS_DIR/test-summary.json" \
  --output "$ARTIFACTS_DIR/verified-tests.json"
