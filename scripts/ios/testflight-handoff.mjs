#!/usr/bin/env node
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const SHA_40_RE = /^[0-9a-f]{40}$/i;
const SHA_64_RE = /^[0-9a-f]{64}$/i;

export function checkCandidateForAction({ candidate, action, headCommit, queuedCommit }) {
  if (!candidate || typeof candidate !== 'object') {
    throw new Error('testflight-candidate.json payload is invalid');
  }
  for (const field of ['bundleBaseId', 'widgetsBundleId', 'appGroupId', 'marketingVersion', 'buildNumber']) {
    if (!candidate[field] || String(candidate[field]).trim() === '' || candidate[field] === 'UNSET') {
      throw new Error(`testflight-candidate.json missing required field: ${field}`);
    }
  }

  if (action === 'build') {
    return {
      action,
      sourceCommitChecked: false,
      bundleBaseId: candidate.bundleBaseId,
      marketingVersion: candidate.marketingVersion,
      buildNumber: candidate.buildNumber
    };
  }

  if (!['upload', 'inspect', 'deliver', 'verify-only'].includes(action)) {
    throw new Error(`Unsupported TestFlight action: ${action}`);
  }

  const manifestCommit = String(candidate.sourceCommit || '').trim();
  const sourceCommit = queuedCommit && queuedCommit !== 'UNSET' ? queuedCommit : manifestCommit;
  if (manifestCommit && manifestCommit !== 'UNSET' && manifestCommit.toLowerCase() !== sourceCommit.toLowerCase()) {
    throw new Error('Queued commit conflicts with the candidate sourceCommit');
  }
  if (!sourceCommit || sourceCommit === 'UNSET' || !SHA_40_RE.test(sourceCommit)) {
    throw new Error('testflight-candidate.json sourceCommit must be a pinned 40-char git commit SHA before upload/deliver');
  }
  const normalizedHead = String(headCommit || '').trim();
  if (!SHA_40_RE.test(normalizedHead) || normalizedHead.toLowerCase() !== sourceCommit.toLowerCase()) {
    throw new Error(`Source commit immutability check failed: HEAD (${normalizedHead}) != release commit (${sourceCommit})`);
  }

  const teamId = String(candidate.teamId || '').trim();
  if (!teamId || teamId === 'UNSET') {
    throw new Error('testflight-candidate.json teamId must not be UNSET');
  }

  const certSha = String(candidate.certificateSha256 || '').trim();
  if (!certSha || certSha === 'UNSET' || !SHA_64_RE.test(certSha)) {
    throw new Error('testflight-candidate.json certificateSha256 must be a 64-char hex SHA-256');
  }

  return {
    action,
    sourceCommitChecked: true,
    sourceCommit: sourceCommit.toLowerCase(),
    teamId,
    certificateSha256: certSha.toLowerCase()
  };
}

export function verifySigningReportMatch({ signingReport, candidate, actualIpaSha256 }) {
  if (!signingReport || signingReport.verified !== true) {
    throw new Error('device-signing.json missing verified=true');
  }
  if (signingReport.teamId !== candidate.teamId) {
    throw new Error(`Signing report teamId mismatch: ${signingReport.teamId} != ${candidate.teamId}`);
  }
  if (String(signingReport.certificateSha256 || '').toLowerCase() !== String(candidate.certificateSha256 || '').toLowerCase()) {
    throw new Error('Signing report certificateSha256 does not match testflight-candidate.json');
  }
  if (String(signingReport.marketingVersion) !== String(candidate.marketingVersion)) {
    throw new Error('Signing report marketingVersion mismatch');
  }
  if (String(signingReport.buildNumber) !== String(candidate.buildNumber)) {
    throw new Error('Signing report buildNumber mismatch');
  }
  if (signingReport.appGroupId !== candidate.appGroupId) {
    throw new Error('Signing report appGroupId mismatch');
  }

  const appBundle = signingReport.bundles?.app;
  const widgetBundle = signingReport.bundles?.widgets;
  if (!appBundle || appBundle.bundleId !== candidate.bundleBaseId || !appBundle.xcarchiveVerified || !appBundle.ipaVerified) {
    throw new Error('Main app bundle failed dual-stage verification in device-signing.json');
  }
  if (
    !widgetBundle ||
    widgetBundle.bundleId !== candidate.widgetsBundleId ||
    !widgetBundle.xcarchiveVerified ||
    !widgetBundle.ipaVerified
  ) {
    throw new Error('Widgets extension bundle failed dual-stage verification in device-signing.json');
  }

  const reportedSha = String(signingReport.ipa?.sha256 || '').toLowerCase();
  if (!SHA_64_RE.test(reportedSha) || reportedSha !== String(actualIpaSha256 || '').toLowerCase()) {
    throw new Error(`IPA SHA-256 mismatch: signing report=${reportedSha}, actual=${actualIpaSha256}`);
  }

  return {
    verified: true,
    ipaSha256: reportedSha,
    bundleBaseId: appBundle.bundleId,
    widgetsBundleId: widgetBundle.bundleId
  };
}

export function readApiIdentifiers(jsonObj) {
  if (!jsonObj || typeof jsonObj !== 'object') {
    throw new Error('apple-api-identifiers.json payload must be a JSON object');
  }
  const keyId = String(jsonObj.keyId || '').trim();
  const issuerId = String(jsonObj.issuerId || '').trim();
  if (!keyId || keyId === 'UNSET') {
    throw new Error('apple-api-identifiers.json missing non-empty keyId');
  }
  if (!issuerId || issuerId === 'UNSET') {
    throw new Error('apple-api-identifiers.json missing non-empty issuerId');
  }
  return { keyId, issuerId };
}

export function computeFileSha256Sync(filePath) {
  const hash = crypto.createHash('sha256');
  hash.update(fs.readFileSync(filePath));
  return hash.digest('hex').toLowerCase();
}

function parseArgs(argv) {
  const out = { action: 'verify-only', candidate: 'ci/testflight-candidate.json' };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--action') out.action = argv[++i];
    else if (arg === '--candidate') out.candidate = argv[++i];
    else if (arg === '--signing-report') out.signingReport = argv[++i];
    else if (arg === '--ipa') out.ipa = argv[++i];
    else if (arg === '--head-commit') out.headCommit = argv[++i];
    else if (arg === '--api-identifiers') out.apiIdentifiers = argv[++i];
    else if (arg === '--api-key-id') out.apiKeyId = argv[++i];
    else if (arg === '--api-issuer-id') out.apiIssuerId = argv[++i];
    else if (arg === '--private-keys-dir') out.privateKeysDir = argv[++i];
    else if (arg === '--output') out.output = argv[++i];
  }
  return out;
}

if (path.resolve(process.argv[1] || '') === path.resolve(fileURLToPath(import.meta.url))) {
  const args = parseArgs(process.argv.slice(2));
  const candidate = JSON.parse(fs.readFileSync(args.candidate, 'utf8'));
  const headCommit =
    args.headCommit ||
    execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();

  const candidateCheck = checkCandidateForAction({
    candidate,
    action: args.action,
    queuedCommit: process.env.TT_RELEASE_COMMIT,
    headCommit
  });

  if (!args.signingReport || !args.ipa) {
    throw new Error('--signing-report and --ipa are required for testflight-handoff');
  }
  const signingReport = JSON.parse(fs.readFileSync(args.signingReport, 'utf8'));
  const actualIpaSha = computeFileSha256Sync(args.ipa);
  const signingCheck = verifySigningReportMatch({
    signingReport,
    candidate,
    actualIpaSha256: actualIpaSha
  });

  let uploadExecuted = false;
  if (args.action === 'upload') {
    const ids = args.apiIdentifiers
      ? readApiIdentifiers(JSON.parse(fs.readFileSync(args.apiIdentifiers, 'utf8')))
      : null;
    const apiKeyId = args.apiKeyId || ids?.keyId || process.env.ASC_API_KEY_ID;
    const apiIssuerId = args.apiIssuerId || ids?.issuerId || process.env.ASC_API_ISSUER_ID;
    const privateKeysDir = args.privateKeysDir || process.env.API_PRIVATE_KEYS_DIR;
    if (!apiKeyId || !apiIssuerId || !privateKeysDir) {
      throw new Error('Missing ASC_API_KEY_ID, ASC_API_ISSUER_ID, or API_PRIVATE_KEYS_DIR for upload');
    }

    execFileSync(
      'xcrun',
      [
        'altool',
        '--upload-app',
        '--type',
        'ios',
        '--file',
        args.ipa,
        '--apiKey',
        apiKeyId,
        '--apiIssuer',
        apiIssuerId
      ],
      {
        stdio: 'inherit',
        env: {
          ...process.env,
          API_PRIVATE_KEYS_DIR: privateKeysDir
        }
      }
    );
    uploadExecuted = true;
  }

  const summary = {
    schemaVersion: 1,
    action: args.action,
    candidate: candidateCheck,
    signing: signingCheck,
    uploadExecuted,
    completedAt: new Date().toISOString()
  };
  if (args.output) {
    fs.mkdirSync(path.dirname(args.output), { recursive: true });
    fs.writeFileSync(args.output, JSON.stringify(summary, null, 2));
  }
  console.log(JSON.stringify(summary, null, 2));
}
