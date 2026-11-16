#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export function verifyTestSummary(summary) {
  if (!summary || typeof summary !== 'object') {
    throw new Error('Test summary JSON is missing or invalid');
  }
  const executed = Number(summary.testsExecuted ?? 0);
  const passed = Number(summary.testsPassed ?? 0);
  const failed = Number(summary.testsFailed ?? 0);
  const skipped = Number(summary.testsSkipped ?? 0);
  const allowedSkips = Array.isArray(summary.allowedSkips) ? summary.allowedSkips : [];
  const unexpectedSkips = Math.max(0, skipped - allowedSkips.length);

  if (executed <= 0) {
    throw new Error(`Expected testsExecuted > 0, got ${executed}`);
  }
  if (failed > 0) {
    throw new Error(`Native iOS test gate failed: testsFailed=${failed}`);
  }
  if (unexpectedSkips > 0) {
    throw new Error(`Native iOS test gate failed: unexpected testsSkipped=${unexpectedSkips}`);
  }
  if (passed <= 0 || passed + skipped !== executed) {
    throw new Error(`Inconsistent native test counts: executed=${executed}, passed=${passed}, skipped=${skipped}`);
  }

  return {
    verified: true,
    executed,
    passed,
    failed,
    skipped,
    unexpectedSkips
  };
}

function parseCliArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--summary') out.summary = argv[++i];
    else if (arg === '--output') out.output = argv[++i];
  }
  return out;
}

if (path.resolve(process.argv[1] || '') === path.resolve(fileURLToPath(import.meta.url))) {
  const args = parseCliArgs(process.argv.slice(2));
  if (!args.summary) {
    throw new Error('Usage: verify-native-test-result.mjs --summary <test-summary.json> [--output <verified.json>]');
  }
  const raw = JSON.parse(fs.readFileSync(args.summary, 'utf8'));
  const verified = verifyTestSummary(raw);
  const payload = { ...raw, ...verified, verifiedAt: new Date().toISOString() };
  if (args.output) {
    fs.mkdirSync(path.dirname(args.output), { recursive: true });
    fs.writeFileSync(args.output, JSON.stringify(payload, null, 2));
  }
  console.log(JSON.stringify(payload, null, 2));
}
