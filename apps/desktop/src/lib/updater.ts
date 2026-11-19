import { create } from 'zustand';
import { relaunch } from '@tauri-apps/plugin-process';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { isTauriRuntime } from './ipc';

export type UpdaterStatus =
  | 'idle'
  | 'checking'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'up_to_date'
  | 'error';

export interface AvailableUpdateInfo {
  version: string;
  currentVersion: string;
  body: string;
  date: string | null;
}

interface UpdaterStoreState {
  status: UpdaterStatus;
  availableUpdate: AvailableUpdateInfo | null;
  downloadedBytes: number;
  totalBytes: number | null;
  lastCheckedAtMs: number | null;
  errorMessage: string | null;
  pendingHandle: Update | null;
  dismissedVersion: string | null;
  setStatePartial: (partial: Partial<UpdaterStoreState>) => void;
  dismissBanner: () => void;
}

export const useUpdaterStore = create<UpdaterStoreState>((set) => ({
  status: 'idle',
  availableUpdate: null,
  downloadedBytes: 0,
  totalBytes: null,
  lastCheckedAtMs: null,
  errorMessage: null,
  pendingHandle: null,
  dismissedVersion: null,
  setStatePartial: (partial) => set(partial),
  dismissBanner: () =>
    set((s) => ({
      dismissedVersion: s.availableUpdate?.version ?? s.dismissedVersion,
    })),
}));

let inFlightCheck: Promise<AvailableUpdateInfo | null> | null = null;
let inFlightInstall: Promise<boolean> | null = null;

function stringifyError(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  return '检查更新失败，请稍后重试';
}

export async function checkForUpdates(manual = false): Promise<AvailableUpdateInfo | null> {
  if (!isTauriRuntime()) {
    if (manual) {
      useUpdaterStore.getState().setStatePartial({
        status: 'up_to_date',
        lastCheckedAtMs: Date.now(),
        errorMessage: null,
      });
    }
    return null;
  }

  if (!manual && typeof navigator !== 'undefined' && navigator.onLine === false) {
    return null;
  }

  if (inFlightCheck) {
    return inFlightCheck;
  }

  const store = useUpdaterStore.getState();
  store.setStatePartial({
    status: 'checking',
    errorMessage: null,
  });

  inFlightCheck = (async () => {
    try {
      const update = await check({ timeout: 15_000 });
      const checkedAt = Date.now();
      if (update) {
        const info: AvailableUpdateInfo = {
          version: update.version,
          currentVersion: update.currentVersion,
          body: update.body?.trim() ?? '',
          date: update.date ?? null,
        };
        const prev = useUpdaterStore.getState().pendingHandle;
        if (prev && prev !== update) {
          prev.close().catch(() => {});
        }
        useUpdaterStore.getState().setStatePartial({
          status: 'available',
          availableUpdate: info,
          pendingHandle: update,
          lastCheckedAtMs: checkedAt,
          errorMessage: null,
          dismissedVersion:
            useUpdaterStore.getState().dismissedVersion === info.version
              ? info.version
              : null,
        });
        return info;
      }

      // Release the native resource behind a stale handle (e.g. the release
      // was pulled) instead of dropping the reference and leaking it.
      useUpdaterStore.getState().pendingHandle?.close().catch(() => {});
      useUpdaterStore.getState().setStatePartial({
        status: manual ? 'up_to_date' : 'idle',
        availableUpdate: null,
        pendingHandle: null,
        lastCheckedAtMs: checkedAt,
        errorMessage: null,
      });
      return null;
    } catch (err) {
      if (manual) {
        useUpdaterStore.getState().setStatePartial({
          status: 'error',
          lastCheckedAtMs: Date.now(),
          errorMessage: stringifyError(err),
        });
      } else {
        // 静默后台检查在网络受限或无 release manifest 时不打扰用户
        useUpdaterStore.getState().setStatePartial({
          status: 'idle',
          errorMessage: null,
        });
      }
      return null;
    } finally {
      inFlightCheck = null;
    }
  })();

  return inFlightCheck;
}

export async function downloadAndInstallUpdate(): Promise<boolean> {
  if (!isTauriRuntime()) {
    return false;
  }
  if (inFlightInstall) {
    return inFlightInstall;
  }

  inFlightInstall = (async () => {
    let handle = useUpdaterStore.getState().pendingHandle;
    if (!handle) {
      await checkForUpdates(true);
      handle = useUpdaterStore.getState().pendingHandle;
    }
    if (!handle) {
      return false;
    }

    useUpdaterStore.getState().setStatePartial({
      status: 'downloading',
      downloadedBytes: 0,
      totalBytes: null,
      errorMessage: null,
    });

    try {
      await handle.downloadAndInstall((event) => {
        const cur = useUpdaterStore.getState();
        if (event.event === 'Started') {
          cur.setStatePartial({
            status: 'downloading',
            downloadedBytes: 0,
            totalBytes: event.data.contentLength ?? null,
          });
        } else if (event.event === 'Progress') {
          cur.setStatePartial({
            status: 'downloading',
            downloadedBytes: cur.downloadedBytes + event.data.chunkLength,
          });
        } else if (event.event === 'Finished') {
          cur.setStatePartial({
            status: 'installing',
          });
        }
      });

      useUpdaterStore.getState().setStatePartial({
        status: 'installing',
      });
      await relaunch();
      return true;
    } catch (err) {
      useUpdaterStore.getState().setStatePartial({
        status: 'available',
        errorMessage: stringifyError(err),
      });
      return false;
    } finally {
      inFlightInstall = null;
    }
  })();

  return inFlightInstall;
}
