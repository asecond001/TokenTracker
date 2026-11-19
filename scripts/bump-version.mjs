#!/usr/bin/env node
// Bump the desktop release version everywhere it is declared, in one step.
//
//   node scripts/bump-version.mjs 0.4.8     # rewrite every manifest
//   node scripts/bump-version.mjs --check   # fail if manifests disagree
//
// The iOS app keeps its own marketing version (apps/ios) and is not touched.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;
const WORKSPACE_CRATES = ['tokvane-collectors', 'tokvane-core', 'tokvane-desktop', 'tokvane-ffi'];
const JSON_MANIFESTS = [
  'package.json',
  'apps/desktop/package.json',
  'apps/desktop/src-tauri/tauri.conf.json',
  'services/sync-worker/package.json',
];

const read = (rel) => readFileSync(join(root, rel), 'utf8');
const write = (rel, text) => writeFileSync(join(root, rel), text);

const jsonVersionRe = /("version"\s*:\s*")([^"]+)(")/;
const cargoWorkspaceRe = /(\[workspace\.package\][^[]*?\nversion\s*=\s*")([^"]+)(")/;
const lockEntryRe = (name) =>
  new RegExp(`(\\[\\[package\\]\\]\\nname = "${name}"\\nversion = ")([^"]+)(")`);

/** Every declared version, keyed by a human-readable location. */
function collectVersions() {
  const found = {};
  for (const rel of JSON_MANIFESTS) {
    found[rel] = read(rel).match(jsonVersionRe)?.[2];
  }
  found['Cargo.toml [workspace.package]'] = read('Cargo.toml').match(cargoWorkspaceRe)?.[2];
  const lock = read('Cargo.lock');
  for (const name of WORKSPACE_CRATES) {
    found[`Cargo.lock ${name}`] = lock.match(lockEntryRe(name))?.[2];
  }
  return found;
}

function replaceOnce(rel, re, version) {
  const text = read(rel);
  if (!re.test(text)) {
    throw new Error(`No version field found in ${rel}`);
  }
  write(rel, text.replace(re, `$1${version}$3`));
}

function check() {
  const found = collectVersions();
  const distinct = new Set(Object.values(found));
  if (distinct.size !== 1 || distinct.has(undefined)) {
    console.error('Version mismatch across manifests:');
    for (const [where, v] of Object.entries(found)) {
      console.error(`  ${where}: ${v ?? '(missing)'}`);
    }
    process.exit(1);
  }
  console.log(`All manifests at ${[...distinct][0]}`);
}

function bump(version) {
  if (!SEMVER.test(version)) {
    console.error(`Not a semver version: ${version}`);
    process.exit(1);
  }
  for (const rel of JSON_MANIFESTS) {
    replaceOnce(rel, jsonVersionRe, version);
  }
  replaceOnce('Cargo.toml', cargoWorkspaceRe, version);
  for (const name of WORKSPACE_CRATES) {
    replaceOnce('Cargo.lock', lockEntryRe(name), version);
  }
  check();
}

const arg = process.argv[2];
if (!arg || arg === '--help' || arg === '-h') {
  console.log('Usage: node scripts/bump-version.mjs <x.y.z> | --check');
  process.exit(arg ? 0 : 1);
} else if (arg === '--check') {
  check();
} else {
  bump(arg.replace(/^v/, ''));
}
