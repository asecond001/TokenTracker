import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { validateIosProject } from '../check-ios-project.mjs';
import { verifyTestSummary } from '../verify-native-test-result.mjs';
import { checkCandidateForAction, readApiIdentifiers, verifySigningReportMatch } from '../testflight-handoff.mjs';
import { assertBuildValidForDelivery, buildAscJwt, selectMatchingBuild } from '../testflight-owner.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '../../..');

function readRepoFile(relPath) {
  return fs.readFileSync(path.join(repoRoot, relPath), 'utf8');
}

test('validateIosProject passes for repository configuration', () => {
  const result = validateIosProject(repoRoot);
  assert.equal(result.ok, true);
  assert.ok(result.checkedFiles >= 18);
  assert.ok(result.localizedKeys >= 15);
});

test('verifyTestSummary accepts strict passing summaries and rejects zero/failed/skipped runs', () => {
  const ok = verifyTestSummary({
    testsExecuted: 9,
    testsPassed: 9,
    testsFailed: 0,
    testsSkipped: 0,
    allowedSkips: []
  });
  assert.equal(ok.verified, true);
  assert.equal(ok.executed, 9);

  assert.throws(() => verifyTestSummary({ testsExecuted: 0, testsPassed: 0, testsFailed: 0, testsSkipped: 0 }));
  assert.throws(() => verifyTestSummary({ testsExecuted: 5, testsPassed: 4, testsFailed: 1, testsSkipped: 0 }));
  assert.throws(() =>
    verifyTestSummary({ testsExecuted: 5, testsPassed: 4, testsFailed: 0, testsSkipped: 1, allowedSkips: [] })
  );
});

test('testflight-handoff enforces sourceCommit immutability and UNSET rejection', () => {
  const commit = 'a'.repeat(40);
  const certSha = 'b'.repeat(64);
  const baseCandidate = {
    bundleBaseId: 'com.tokentracker.app',
    widgetsBundleId: 'com.tokentracker.app.widgets',
    appGroupId: 'group.com.tokentracker.app',
    marketingVersion: '0.3.0',
    buildNumber: '1',
    sourceCommit: 'UNSET',
    teamId: 'UNSET',
    certificateSha256: 'UNSET'
  };

  assert.equal(checkCandidateForAction({ candidate: baseCandidate, action: 'build' }).sourceCommitChecked, false);
  assert.throws(() => checkCandidateForAction({ candidate: baseCandidate, action: 'upload', headCommit: commit }));

  const pinnedCandidate = {
    ...baseCandidate,
    sourceCommit: commit,
    teamId: 'TEAM123456',
    certificateSha256: certSha
  };
  assert.equal(
    checkCandidateForAction({ candidate: pinnedCandidate, action: 'upload', headCommit: commit }).sourceCommit,
    commit
  );
  assert.throws(() =>
    checkCandidateForAction({
      candidate: pinnedCandidate,
      action: 'upload',
      headCommit: 'c'.repeat(40)
    })
  );
});

test('verifySigningReportMatch checks both .xcarchive/.ipa bundles and exact IPA sha256', () => {
  const ipaSha = 'd'.repeat(64);
  const certSha = 'e'.repeat(64);
  const candidate = {
    bundleBaseId: 'com.tokentracker.app',
    widgetsBundleId: 'com.tokentracker.app.widgets',
    appGroupId: 'group.com.tokentracker.app',
    marketingVersion: '0.3.0',
    buildNumber: '7',
    teamId: 'TEAM123456',
    certificateSha256: certSha
  };
  const report = {
    verified: true,
    teamId: 'TEAM123456',
    certificateSha256: certSha,
    marketingVersion: '0.3.0',
    buildNumber: '7',
    appGroupId: 'group.com.tokentracker.app',
    bundles: {
      app: { bundleId: 'com.tokentracker.app', xcarchiveVerified: true, ipaVerified: true },
      widgets: { bundleId: 'com.tokentracker.app.widgets', xcarchiveVerified: true, ipaVerified: true }
    },
    ipa: { filename: 'TokenTracker.ipa', sizeBytes: 1024, sha256: ipaSha }
  };

  const matched = verifySigningReportMatch({ signingReport: report, candidate, actualIpaSha256: ipaSha });
  assert.equal(matched.verified, true);
  assert.equal(matched.ipaSha256, ipaSha);

  assert.throws(() =>
    verifySigningReportMatch({ signingReport: report, candidate, actualIpaSha256: 'f'.repeat(64) })
  );
  assert.throws(() =>
    verifySigningReportMatch({
      signingReport: {
        ...report,
        bundles: {
          ...report.bundles,
          widgets: { bundleId: 'com.tokentracker.app.widgets', xcarchiveVerified: true, ipaVerified: false }
        }
      },
      candidate,
      actualIpaSha256: ipaSha
    })
  );
});

test('testflight-owner signs verifiable ES256 ASC JWT and enforces VALID processingState', () => {
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ec', {
    namedCurve: 'prime256v1'
  });
  const privatePem = privateKey.export({ type: 'pkcs8', format: 'pem' }).toString();
  const jwt = buildAscJwt({
    keyId: 'KEYID123',
    issuerId: '00000000-0000-0000-0000-000000000001',
    privateKeyPem: privatePem,
    nowSec: 1_760_000_000
  });
  const parts = jwt.split('.');
  assert.equal(parts.length, 3);
  const sigBuf = Buffer.from(parts[2].replace(/-/g, '+').replace(/_/g, '/'), 'base64');
  assert.equal(sigBuf.length, 64);
  const verified = crypto.verify(
    'sha256',
    Buffer.from(`${parts[0]}.${parts[1]}`, 'utf8'),
    { key: publicKey, dsaEncoding: 'ieee-p1363' },
    sigBuf
  );
  assert.equal(verified, true);

  const selected = selectMatchingBuild({
    builds: [
      {
        id: 'b-1',
        attributes: { version: '7', processingState: 'VALID', uploadedDate: '2026-11-16T00:00:00Z' },
        relationships: { preReleaseVersion: { data: { id: 'prv-1' } } }
      }
    ],
    included: [
      { type: 'preReleaseVersions', id: 'prv-1', attributes: { version: '0.3.0' } }
    ],
    marketingVersion: '0.3.0',
    buildNumber: '7'
  });
  assert.equal(selected.id, 'b-1');
  assert.equal(assertBuildValidForDelivery(selected), true);
  assert.throws(() => assertBuildValidForDelivery({ id: 'b-2', processingState: 'PROCESSING' }));
});

test('Azure pipeline YAMLs, secure files, and shell scripts enforce manual triggers, Xcode pinning, and offline wheelhouse flags', () => {
  const ciYaml = readRepoFile('azure-pipelines.yml');
  const relYaml = readRepoFile('azure-pipelines-release.yml');
  const setupSh = readRepoFile('scripts/ios/azure-setup.sh');
  const wheelhousePy = readRepoFile('scripts/ios/apple-wheelhouse.py');
  const toolchain = JSON.parse(readRepoFile('ci/ios-toolchain.json'));

  for (const [name, yml] of [
    ['azure-pipelines.yml', ciYaml],
    ['azure-pipelines-release.yml', relYaml]
  ]) {
    assert.ok(yml.includes('trigger: none'), `${name} must set trigger: none`);
    assert.ok(yml.includes('pr: none'), `${name} must set pr: none`);
    assert.ok(yml.includes(`vmImage: ${toolchain.vmImage}`), `${name} must target ${toolchain.vmImage}`);
  }

  for (const secureFile of [
    'tokentracker_distribution.p12',
    'tokentracker_p12_password.txt',
    'tokentracker_app.mobileprovision',
    'tokentracker_widgets.mobileprovision',
    'tokentracker_appstore.p8',
    'apple-api-identifiers.json'
  ]) {
    assert.ok(relYaml.includes(secureFile), `azure-pipelines-release.yml missing Secure File ${secureFile}`);
  }
  assert.ok(!relYaml.includes('InstallAppleCertificate'), 'Must not use broken InstallAppleCertificate@2 task');
  assert.ok(!relYaml.includes('Xcode@5'), 'Must not use Xcode@5 multi-profile limitation');

  const ids = readApiIdentifiers({ keyId: 'ABC1234567', issuerId: '11111111-2222-3333-4444-555555555555' });
  assert.equal(ids.keyId, 'ABC1234567');
  assert.throws(() => readApiIdentifiers({ keyId: 'UNSET', issuerId: 'uuid' }));

  assert.ok(setupSh.includes('xcode-select -s'), 'azure-setup.sh must pin Xcode via xcode-select -s');
  assert.ok(setupSh.includes('DEVELOPER_DIR'), 'azure-setup.sh must export DEVELOPER_DIR');
  assert.ok(wheelhousePy.includes('--no-index'), 'apple-wheelhouse.py must install offline via --no-index');
  assert.ok(wheelhousePy.includes('--only-binary=:all:'), 'apple-wheelhouse.py must enforce --only-binary=:all:');
});
