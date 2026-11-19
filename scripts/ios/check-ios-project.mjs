#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '../..');

function readText(relPath) {
  return fs.readFileSync(path.join(repoRoot, relPath), 'utf8');
}

function assert(cond, msg) {
  if (!cond) {
    throw new Error(msg);
  }
}

export function validateIosProject(rootDir = repoRoot) {
  const errors = [];
  const check = (cond, msg) => {
    if (!cond) errors.push(msg);
  };

  // iOS is versioned independently of the desktop/workspace package.json;
  // testflight-candidate.json is the iOS marketing version source of truth.
  const candidate = JSON.parse(fs.readFileSync(path.join(rootDir, 'ci/testflight-candidate.json'), 'utf8'));
  const toolchain = JSON.parse(fs.readFileSync(path.join(rootDir, 'ci/ios-toolchain.json'), 'utf8'));

  const projectYml = fs.readFileSync(path.join(rootDir, 'apps/ios/project.yml'), 'utf8');
  check(projectYml.includes('BUNDLE_BASE_ID: com.asecond.tokenvane'), 'project.yml missing BUNDLE_BASE_ID: com.asecond.tokenvane');
  check(projectYml.includes('APP_GROUP_ID: group.$(BUNDLE_BASE_ID)'), 'project.yml missing APP_GROUP_ID: group.$(BUNDLE_BASE_ID)');
  const iosVersion = String(candidate.marketingVersion ?? '');
  check(/^\d+\.\d+\.\d+$/.test(iosVersion), `testflight-candidate.json marketingVersion must be X.Y.Z (got "${iosVersion}")`);
  check(projectYml.includes(`MARKETING_VERSION: "${iosVersion}"`), `project.yml MARKETING_VERSION must equal testflight-candidate.json marketingVersion (${iosVersion})`);
  check(projectYml.includes(`SWIFT_VERSION: "${toolchain.swiftVersion}"`), `project.yml SWIFT_VERSION must equal ${toolchain.swiftVersion}`);
  check(projectYml.includes(`IPHONEOS_DEPLOYMENT_TARGET: "${toolchain.deploymentTarget}"`), `project.yml deployment target must equal ${toolchain.deploymentTarget}`);
  check(projectYml.includes('PRODUCT_BUNDLE_IDENTIFIER: $(BUNDLE_BASE_ID)'), 'TokenVane target missing $(BUNDLE_BASE_ID)');
  check(projectYml.includes('PRODUCT_BUNDLE_IDENTIFIER: $(BUNDLE_BASE_ID).widgets'), 'TokenVaneWidgets target missing $(BUNDLE_BASE_ID).widgets');
  check(projectYml.includes('SUPPORTS_LIVE_ACTIVITIES: YES'), 'project.yml missing SUPPORTS_LIVE_ACTIVITIES: YES');

  // Verify TokenVaneWidgets block does NOT depend on TokenVaneCore (prevent 30 MB Jetsam & 0xdead10cc)
  const widgetsSectionMatch = projectYml.match(/TokenVaneWidgets:[\s\S]*?(?=\n  TokenVaneTests:)/);
  check(Boolean(widgetsSectionMatch), 'Could not isolate TokenVaneWidgets target in project.yml');
  if (widgetsSectionMatch) {
    check(!widgetsSectionMatch[0].includes('TokenVaneCore'), 'TokenVaneWidgets must NOT link TokenVaneCore / Rust XCFramework');
    check(widgetsSectionMatch[0].includes('path: Widgets'), 'TokenVaneWidgets must include Widgets sources');
    check(widgetsSectionMatch[0].includes('path: Shared'), 'TokenVaneWidgets must include Shared sources');
  }

  // Plists & entitlements
  const appPlist = fs.readFileSync(path.join(rootDir, 'apps/ios/App/Info.plist'), 'utf8');
  check(appPlist.includes('<key>NSSupportsLiveActivities</key>'), 'App/Info.plist missing NSSupportsLiveActivities');
  check(appPlist.includes('<key>BGTaskSchedulerPermittedIdentifiers</key>'), 'App/Info.plist missing BGTaskSchedulerPermittedIdentifiers');
  check(appPlist.includes('$(BUNDLE_BASE_ID).sync-refresh'), 'App/Info.plist missing sync-refresh task identifier');
  check(appPlist.includes('<key>ITSAppUsesNonExemptEncryption</key>'), 'App/Info.plist missing ITSAppUsesNonExemptEncryption');

  const widgetPlist = fs.readFileSync(path.join(rootDir, 'apps/ios/Widgets/Info.plist'), 'utf8');
  check(widgetPlist.includes('com.apple.widgetkit-extension'), 'Widgets/Info.plist missing com.apple.widgetkit-extension');

  for (const entRel of ['apps/ios/App/TokenVane.entitlements', 'apps/ios/Widgets/TokenVaneWidgets.entitlements']) {
    const ent = fs.readFileSync(path.join(rootDir, entRel), 'utf8');
    check(ent.includes('$(APP_GROUP_ID)'), `${entRel} missing $(APP_GROUP_ID)`);
  }

  for (const privRel of ['apps/ios/App/Resources/PrivacyInfo.xcprivacy', 'apps/ios/Widgets/PrivacyInfo.xcprivacy']) {
    const priv = fs.readFileSync(path.join(rootDir, privRel), 'utf8');
    check(priv.includes('NSPrivacyAccessedAPICategoryFileTimestamp'), `${privRel} missing FileTimestamp reason`);
    check(priv.includes('<string>C617.1</string>'), `${privRel} missing C617.1`);
    check(priv.includes('<key>NSPrivacyTracking</key>'), `${privRel} missing NSPrivacyTracking`);
  }

  // Localization completeness
  const loc = JSON.parse(fs.readFileSync(path.join(rootDir, 'apps/ios/App/Resources/Localizable.xcstrings'), 'utf8'));
  const locEntries = Object.entries(loc.strings || {});
  check(locEntries.length >= 15, 'Localizable.xcstrings expected at least 15 entries');
  for (const [key, entry] of locEntries) {
    check(Boolean(entry?.localizations?.en?.stringUnit?.value), `Missing en localization for ${key}`);
    check(Boolean(entry?.localizations?.['zh-Hans']?.stringUnit?.value), `Missing zh-Hans localization for ${key}`);
  }

  // CoreFFI Package.swift & AppIcon PNG magic check
  const pkgSwift = fs.readFileSync(path.join(rootDir, 'apps/ios/CoreFFI/Package.swift'), 'utf8');
  check(pkgSwift.includes('Artifacts/TokenVaneCoreFFI.xcframework'), 'CoreFFI/Package.swift missing xcframework binaryTarget');
  check(pkgSwift.includes('swiftLanguageModes: [.v5]'), 'CoreFFI/Package.swift must select Swift 5 language mode');

  const appIconPath = path.join(rootDir, 'apps/ios/App/Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png');
  check(fs.existsSync(appIconPath), 'AppIcon.png missing');
  if (fs.existsSync(appIconPath)) {
    const iconBytes = fs.readFileSync(appIconPath);
    const pngSig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
    check(iconBytes.subarray(0, 8).equals(pngSig) && iconBytes.length > 1000, 'AppIcon.png is not a valid non-empty PNG');
  }

  // Required Swift sources + zero WebView check
  const requiredSwift = [
    'apps/ios/Shared/AppGroupPaths.swift',
    'apps/ios/Shared/KeychainStore.swift',
    'apps/ios/Shared/WidgetSnapshotModels.swift',
    'apps/ios/Shared/WidgetRefreshBridge.swift',
    'apps/ios/Shared/CodingSessionAttributes.swift',
    'apps/ios/Shared/L10n.swift',
    'apps/ios/Shared/Formatters.swift',
    'apps/ios/CoreFFI/Sources/TokenVaneCore/TokenVaneCore.swift',
    'apps/ios/App/TokenVaneApp.swift',
    'apps/ios/App/Models/CoreReportDTOs.swift',
    'apps/ios/App/Models/ModelComparisonBuilder.swift',
    'apps/ios/App/Models/ReceiptModelBuilder.swift',
    'apps/ios/App/Services/CoreClientActor.swift',
    'apps/ios/App/Services/QuotaResetNotifier.swift',
    'apps/ios/App/Services/SSEFrameParser.swift',
    'apps/ios/App/Services/SyncCoordinator.swift',
    'apps/ios/App/Services/LiveActivityManager.swift',
    'apps/ios/App/State/AppStore.swift',
    'apps/ios/App/Theme/DesignSystem.swift',
    'apps/ios/App/Views/Components/TimeRangePickerSheet.swift',
    'apps/ios/App/Views/RootSplitOrTabView.swift',
    'apps/ios/App/Views/Overview/OverviewView.swift',
    'apps/ios/App/Views/Analytics/AnalyticsView.swift',
    'apps/ios/App/Views/Analytics/ShareReceiptSheet.swift',
    'apps/ios/App/Views/Models/ModelsCompareView.swift',
    'apps/ios/App/Views/QuotasAndModels/QuotasAndModelsView.swift',
    'apps/ios/App/Views/Settings/SettingsView.swift',
    'apps/ios/App/Views/Settings/QRScannerSheet.swift',
    'apps/ios/Widgets/TokenVaneWidgetBundle.swift',
    'apps/ios/Widgets/RefreshWidgetIntent.swift',
    'apps/ios/Widgets/OverviewHomeWidget.swift',
    'apps/ios/Widgets/QuotaLockScreenWidget.swift',
    'apps/ios/Widgets/CodingLiveActivityWidget.swift',
    'apps/ios/Tests/TokenVaneTests.swift',
    'apps/ios/UITests/TokenVaneUITests.swift'
  ];
  for (const rel of requiredSwift) {
    const abs = path.join(rootDir, rel);
    check(fs.existsSync(abs), `Missing required Swift source: ${rel}`);
    if (fs.existsSync(abs)) {
      const src = fs.readFileSync(abs, 'utf8');
      check(!src.includes('WKWebView') && !src.includes('WebKit'), `${rel} violates zero-WebView rule`);
    }
  }

  check(candidate.bundleBaseId === 'com.asecond.tokenvane', 'testflight-candidate.json bundleBaseId mismatch');
  check(candidate.widgetsBundleId === 'com.asecond.tokenvane.widgets', 'testflight-candidate.json widgetsBundleId mismatch');
  check(candidate.appGroupId === 'group.com.asecond.tokenvane', 'testflight-candidate.json appGroupId mismatch');

  if (errors.length > 0) {
    throw new Error(`iOS project validation failed:\n - ${errors.join('\n - ')}`);
  }
  return {
    ok: true,
    version: iosVersion,
    checkedFiles: requiredSwift.length,
    localizedKeys: locEntries.length
  };
}

if (path.resolve(process.argv[1] || '') === path.resolve(fileURLToPath(import.meta.url))) {
  const result = validateIosProject();
  console.log(JSON.stringify(result, null, 2));
}
