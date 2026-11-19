import { describe, expect, it } from 'vitest';
import packageJson from '../../package.json';
import tauriConf from '../../src-tauri/tauri.conf.json';
import { APP_VERSION } from '../lib/appVersion';

describe('APP_VERSION', () => {
  it('is a semver string sourced from package.json', () => {
    expect(APP_VERSION).toMatch(/^\d+\.\d+\.\d+/);
    expect(APP_VERSION).toBe(packageJson.version);
  });

  it('matches the Tauri bundle version shipped to the updater', () => {
    expect(APP_VERSION).toBe(tauriConf.version);
  });
});
