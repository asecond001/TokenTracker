import { version } from '../../package.json';

/**
 * Desktop app version, sourced from `apps/desktop/package.json` at build time.
 *
 * `scripts/bump-version.mjs` keeps package.json, tauri.conf.json and the Cargo
 * workspace in lockstep, so UI labels never drift from the shipped binary.
 */
export const APP_VERSION: string = version;
