#!/usr/bin/env node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { validateIosProject } from './check-ios-project.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, '../..');

function resolveCargoBin() {
  const probe = spawnSync('cargo', ['--version'], { encoding: 'utf8' });
  if (probe.status === 0) return 'cargo';
  const candidates =
    process.platform === 'win32'
      ? [path.join(os.homedir(), '.cargo', 'bin', 'cargo.exe')]
      : [path.join(os.homedir(), '.cargo', 'bin', 'cargo')];
  for (const cand of candidates) {
    if (fs.existsSync(cand)) return cand;
  }
  throw new Error('Could not find cargo binary on PATH or in ~/.cargo/bin');
}

function runStep(label, cmd, args, cwd = repoRoot) {
  console.log(`\n[local-check] ==> ${label}`);
  execFileSync(cmd, args, { cwd, stdio: 'inherit' });
}

function findUsablePython() {
  const candidates = [
    ['python3', []],
    ['python', []],
    ['py', ['-3']]
  ];
  for (const [bin, prefixArgs] of candidates) {
    const res = spawnSync(
      bin,
      [...prefixArgs, '-c', 'import sys, cryptography; print(sys.executable)'],
      { encoding: 'utf8' }
    );
    if (res.status === 0) {
      const exePath = (res.stdout || '').trim();
      if (exePath && !exePath.toLowerCase().includes('windowsapps')) {
        return { bin, prefixArgs, exePath };
      }
    }
  }
  return null;
}

function findCompiledFfiLibrary() {
  const debugDir = path.join(repoRoot, 'target', 'debug');
  const names =
    process.platform === 'win32'
      ? ['tokentracker_ffi.dll']
      : process.platform === 'darwin'
        ? ['libtokentracker_ffi.dylib']
        : ['libtokentracker_ffi.so'];
  for (const name of names) {
    const full = path.join(debugDir, name);
    if (fs.existsSync(full)) return full;
  }
  throw new Error(`Compiled tokentracker-ffi library not found in ${debugDir}`);
}

function verifyGeneratedSwiftBindings(swiftFilePath) {
  if (!fs.existsSync(swiftFilePath)) {
    throw new Error(`Generated Swift binding missing: ${swiftFilePath}`);
  }
  const generated = fs.readFileSync(swiftFilePath, 'utf8');
  const requiredSymbols = [
    'MobileCoreEngine',
    'static func `open`(dbPath:',
    'func getBootstrapJson(',
    'func ingestPullResponseJson(',
    'func queryOverviewJson(',
    'func queryTrendsJson(',
    'func queryMonthlyJson(',
    'func queryModelsPricingJson(',
    'func querySpeedLatencyJson(',
    'func queryRemoteSubscriptionsJson(',
    'func buildWidgetSnapshotJson(',
    'func exportMonthlyReport(',
    'func updateMobilePreferencesJson(',
    'func updateSyncConfig(',
    'func resetSyncWatermark(',
    'func applyExchangeRatePayload(',
    'func checkpointForSuspend('
  ];
  for (const sym of requiredSymbols) {
    if (!generated.includes(sym)) {
      throw new Error(`Generated TokenTrackerCore.swift missing expected symbol: ${sym}`);
    }
  }

  const checkedInSwift = path.join(
    repoRoot,
    'apps',
    'ios',
    'CoreFFI',
    'Sources',
    'TokenTrackerCore',
    'TokenTrackerCore.swift'
  );
  if (!fs.existsSync(checkedInSwift)) {
    throw new Error(`Checked-in Swift wrapper missing: ${checkedInSwift}`);
  }
  const checkedIn = fs.readFileSync(checkedInSwift, 'utf8');
  if (checkedIn.replace(/\r\n/g, '\n') !== generated.replace(/\r\n/g, '\n')) {
    throw new Error(
      `Checked-in ${checkedInSwift} is out of sync with UniFFI output (${swiftFilePath})`
    );
  }
}

const args = new Set(process.argv.slice(2));
const skipRust = args.has('--skip-rust');
const requirePython = args.has('--require-python') || process.env.CI === 'true';
const offlineFlag = args.has('--online') ? [] : ['--offline'];

console.log('[local-check] Starting Phase 3A iOS + FFI local verification');

const projectSummary = validateIosProject(repoRoot);
console.log('[local-check] iOS project manifest check OK:', JSON.stringify(projectSummary));

if (!skipRust) {
  const cargo = resolveCargoBin();
  runStep('Clippy core + ffi', cargo, [
    'clippy',
    '-p',
    'tokentracker-core',
    '-p',
    'tokentracker-ffi',
    '--all-targets',
    ...offlineFlag,
    '--',
    '-D',
    'warnings'
  ]);
  runStep('Cargo test core + ffi', cargo, [
    'test',
    '-p',
    'tokentracker-core',
    '-p',
    'tokentracker-ffi',
    ...offlineFlag
  ]);
  runStep('Build tokentracker-ffi host library', cargo, ['build', '-p', 'tokentracker-ffi', ...offlineFlag]);

  const hostLib = findCompiledFfiLibrary();
  const outDir = path.join(repoRoot, 'artifacts', 'ffi-swift-local');
  fs.mkdirSync(outDir, { recursive: true });
  runStep('Generate UniFFI Swift bindings', cargo, [
    'run',
    '-p',
    'tokentracker-ffi',
    ...offlineFlag,
    '--bin',
    'uniffi-bindgen',
    '--',
    'generate',
    '--library',
    hostLib,
    '--language',
    'swift',
    '--out-dir',
    outDir
  ]);
  verifyGeneratedSwiftBindings(path.join(outDir, 'TokenTrackerCore.swift'));
  console.log('[local-check] UniFFI Swift binding signature & sync check OK');
}

runStep('Node CI/TestFlight contract tests', process.execPath, [
  '--test',
  'scripts/ios/test/pipelines-and-handoff.test.mjs'
]);

const python = findUsablePython();
if (python) {
  runStep(
    `Python signing unit tests (${python.exePath})`,
    python.bin,
    [...python.prefixArgs, '-m', 'unittest', 'discover', '-s', 'scripts/ios/test', '-p', 'test_*.py', '-v']
  );
} else if (requirePython) {
  throw new Error('Python 3 with cryptography package is required (--require-python / CI=true) but was not found');
} else {
  console.log(
    '[local-check] Skipping Python signing unit tests (no usable non-WindowsApps Python 3 with cryptography found locally)'
  );
}

console.log('\n[local-check] ALL CHECKS COMPLETED SUCCESSFULLY');
