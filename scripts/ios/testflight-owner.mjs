#!/usr/bin/env node
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { readApiIdentifiers } from './testflight-handoff.mjs';

export function base64UrlEncode(input) {
  const buf = Buffer.isBuffer(input) ? input : Buffer.from(String(input), 'utf8');
  return buf.toString('base64').replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '');
}

export function buildAscJwt({ keyId, issuerId, privateKeyPem, nowSec = Math.floor(Date.now() / 1000) }) {
  if (!keyId || !issuerId || !privateKeyPem) {
    throw new Error('keyId, issuerId, and privateKeyPem are required to sign an App Store Connect JWT');
  }
  const header = base64UrlEncode(JSON.stringify({ alg: 'ES256', kid: keyId, typ: 'JWT' }));
  const payload = base64UrlEncode(
    JSON.stringify({
      iss: issuerId,
      iat: nowSec,
      exp: nowSec + 1200,
      aud: 'appstoreconnect-v1'
    })
  );
  const signingInput = `${header}.${payload}`;
  const signature = crypto.sign('sha256', Buffer.from(signingInput, 'utf8'), {
    key: privateKeyPem,
    dsaEncoding: 'ieee-p1363'
  });
  return `${signingInput}.${base64UrlEncode(signature)}`;
}

export function selectMatchingBuild({ builds, marketingVersion, buildNumber, included = [] }) {
  if (!Array.isArray(builds)) {
    throw new Error('Expected builds array from App Store Connect API');
  }
  const preReleaseMap = new Map();
  for (const item of included) {
    if (item?.type === 'preReleaseVersions' && item.id) {
      preReleaseMap.set(item.id, String(item.attributes?.version || ''));
    }
  }

  for (const b of builds) {
    const version = String(b?.attributes?.version || '');
    if (version !== String(buildNumber)) {
      continue;
    }
    const relId = b?.relationships?.preReleaseVersion?.data?.id;
    if (relId && preReleaseMap.has(relId) && marketingVersion) {
      if (preReleaseMap.get(relId) !== String(marketingVersion)) {
        continue;
      }
    }
    return {
      id: b.id,
      version,
      processingState: String(b?.attributes?.processingState || 'UNKNOWN'),
      uploadedDate: b?.attributes?.uploadedDate || null
    };
  }
  throw new Error(`No TestFlight build found matching marketingVersion=${marketingVersion} buildNumber=${buildNumber}`);
}

export function assertBuildValidForDelivery(build) {
  if (!build || build.processingState !== 'VALID') {
    throw new Error(
      `TestFlight build ${build?.id || 'unknown'} is not ready for delivery: processingState=${build?.processingState}`
    );
  }
  return true;
}

export async function ascRequest({ url, jwt, method = 'GET', body, fetchImpl = globalThis.fetch }) {
  if (typeof fetchImpl !== 'function') {
    throw new Error('Global fetch implementation unavailable');
  }
  const headers = {
    Authorization: `Bearer ${jwt}`,
    Accept: 'application/json'
  };
  const init = { method, headers };
  if (body !== undefined) {
    headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(body);
  }
  const res = await fetchImpl(url, init);
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`App Store Connect API ${method} ${url} failed with HTTP ${res.status}: ${text.slice(0, 400)}`);
  }
  return text ? JSON.parse(text) : null;
}

async function resolveAppId({ bundleBaseId, jwt, baseUrl }) {
  const res = await ascRequest({
    url: `${baseUrl}/apps?filter[bundleId]=${encodeURIComponent(bundleBaseId)}`,
    jwt
  });
  const app = res?.data?.[0];
  if (!app?.id) {
    throw new Error(`App Store Connect app not found for bundleId=${bundleBaseId}`);
  }
  return app.id;
}

async function findBuildWithPolling({ appId, marketingVersion, buildNumber, jwt, baseUrl, timeoutSec }) {
  const deadline = Date.now() + timeoutSec * 1000;
  let lastBuild = null;
  while (Date.now() <= deadline) {
    const res = await ascRequest({
      url: `${baseUrl}/builds?filter[app]=${encodeURIComponent(appId)}&filter[version]=${encodeURIComponent(String(buildNumber))}&include=preReleaseVersion&limit=10`,
      jwt
    });
    try {
      const build = selectMatchingBuild({
        builds: res?.data || [],
        marketingVersion,
        buildNumber,
        included: res?.included || []
      });
      lastBuild = build;
      if (build.processingState === 'VALID') {
        return build;
      }
      if (build.processingState === 'FAILED' || build.processingState === 'INVALID') {
        throw new Error(`TestFlight build ${build.id} failed Apple processing: ${build.processingState}`);
      }
    } catch (err) {
      if (Date.now() + 5000 > deadline) {
        throw err;
      }
    }
    await delay(10000);
  }
  throw new Error(
    `Timed out waiting for TestFlight build ${buildNumber} to become VALID (lastState=${lastBuild?.processingState || 'NOT_FOUND'})`
  );
}

async function assignBuildToGroup({ appId, buildId, groupName, jwt, baseUrl }) {
  const groupsRes = await ascRequest({
    url: `${baseUrl}/betaGroups?filter[app]=${encodeURIComponent(appId)}&filter[name]=${encodeURIComponent(groupName)}`,
    jwt
  });
  const group = groupsRes?.data?.[0];
  if (!group?.id) {
    throw new Error(`TestFlight betaGroup named "${groupName}" not found for app ${appId}`);
  }
  await ascRequest({
    url: `${baseUrl}/betaGroups/${encodeURIComponent(group.id)}/relationships/builds`,
    method: 'POST',
    jwt,
    body: {
      data: [{ type: 'builds', id: buildId }]
    }
  });
  return { groupId: group.id, groupName };
}

function parseArgs(argv) {
  const out = {
    mode: 'inspect',
    candidate: 'ci/testflight-candidate.json',
    timeoutSec: 600,
    baseUrl: 'https://api.appstoreconnect.apple.com/v1'
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--mode') out.mode = argv[++i];
    else if (arg === '--candidate') out.candidate = argv[++i];
    else if (arg === '--api-identifiers') out.apiIdentifiers = argv[++i];
    else if (arg === '--key-id') out.keyId = argv[++i];
    else if (arg === '--issuer-id') out.issuerId = argv[++i];
    else if (arg === '--private-key-path') out.privateKeyPath = argv[++i];
    else if (arg === '--output') out.output = argv[++i];
    else if (arg === '--timeout-sec') out.timeoutSec = Number(argv[++i]);
    else if (arg === '--base-url') out.baseUrl = argv[++i];
  }
  return out;
}

if (path.resolve(process.argv[1] || '') === path.resolve(fileURLToPath(import.meta.url))) {
  const args = parseArgs(process.argv.slice(2));
  if (!['inspect', 'deliver'].includes(args.mode)) {
    throw new Error(`Unsupported testflight-owner mode: ${args.mode}`);
  }
  const candidate = JSON.parse(fs.readFileSync(args.candidate, 'utf8'));
  const ids = args.apiIdentifiers
    ? readApiIdentifiers(JSON.parse(fs.readFileSync(args.apiIdentifiers, 'utf8')))
    : null;
  const keyId = args.keyId || ids?.keyId || process.env.ASC_API_KEY_ID;
  const issuerId = args.issuerId || ids?.issuerId || process.env.ASC_API_ISSUER_ID;
  const privateKeyPem = args.privateKeyPath
    ? fs.readFileSync(args.privateKeyPath, 'utf8')
    : process.env.ASC_PRIVATE_KEY_P8;

  const jwt = buildAscJwt({ keyId, issuerId, privateKeyPem });
  const appId = await resolveAppId({
    bundleBaseId: candidate.bundleBaseId,
    jwt,
    baseUrl: args.baseUrl
  });
  const build = await findBuildWithPolling({
    appId,
    marketingVersion: candidate.marketingVersion,
    buildNumber: candidate.buildNumber,
    jwt,
    baseUrl: args.baseUrl,
    timeoutSec: args.mode === 'deliver' ? Math.min(args.timeoutSec, 60) : args.timeoutSec
  });
  assertBuildValidForDelivery(build);

  let delivery = null;
  if (args.mode === 'deliver') {
    delivery = await assignBuildToGroup({
      appId,
      buildId: build.id,
      groupName: candidate.internalTestingGroup || 'Owner Testing',
      jwt,
      baseUrl: args.baseUrl
    });
  }

  const report = {
    schemaVersion: 1,
    mode: args.mode,
    appId,
    bundleBaseId: candidate.bundleBaseId,
    build,
    delivery,
    completedAt: new Date().toISOString()
  };
  if (args.output) {
    fs.mkdirSync(path.dirname(args.output), { recursive: true });
    fs.writeFileSync(args.output, JSON.stringify(report, null, 2));
  }
  console.log(JSON.stringify(report, null, 2));
}
