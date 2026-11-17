import { DatabaseSync } from 'node:sqlite';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  SyncHubCore,
  type SqlAdapter,
  type SyncEnvelopeV1,
  type SyncPullResponseV1,
} from '../hub.js';
import { handleWorkerFetch, type WorkerEnv } from '../index.js';

class NodeSqliteAdapter implements SqlAdapter {
  private readonly db: DatabaseSync;

  constructor(db: DatabaseSync) {
    this.db = db;
  }

  exec<T extends Record<string, unknown> = Record<string, unknown>>(
    query: string,
    ...bindings: Array<string | number | null>
  ): T[] {
    const trimmed = query.trim();
    const upper = trimmed.toUpperCase();
    if (bindings.length === 0 && trimmed.includes(';')) {
      this.db.exec(trimmed);
      return [];
    }
    const stmt = this.db.prepare(trimmed);
    if (upper.startsWith('SELECT') || upper.startsWith('PRAGMA') || upper.startsWith('WITH')) {
      return stmt.all(...bindings) as T[];
    }
    stmt.run(...bindings);
    return [];
  }
}

function makeTestEnv(hub: SyncHubCore, token = 'super-secret-sync-token-2026'): WorkerEnv {
  return {
    SYNC_TOKEN: token,
    SYNC_HUB: {
      idFromName: (name: string) => ({ toString: () => name }),
      get: () => ({
        fetch: (input: RequestInfo | URL, init?: RequestInit) => {
          const req = input instanceof Request ? input : new Request(input, init);
          return hub.handleHubRequest(req);
        },
      }),
    },
  };
}

function makeEnvelope(deviceId: string, displayName: string, tokens: number): SyncEnvelopeV1 {
  return {
    schema_version: 1,
    device_id: deviceId,
    device_display_name: displayName,
    generated_at_ms: 1_776_000_000_000,
    since_watermark_ms: 0,
    hourly_deltas: [
      {
        provider_id: 'claude_code',
        model_id: 'claude-sonnet-4-5',
        bucket_start_ms: 1_776_000_000_000,
        request_count: 3,
        observable_step_count: 3,
        input_tokens: tokens,
        output_tokens: 450,
        cache_read_tokens: 200,
        cache_write_tokens: 0,
        reasoning_tokens: 0,
        estimated_cost_micro_usd: 15_000,
        cache_saved_micro_usd: 540,
        total_duration_ms: 4_200,
        tps_sample_count: 3,
        tps_sum: 180.0,
        max_output_tps: 72.5,
        updated_at_ms: 1_776_000_000_500,
      },
    ],
    subscription_snapshots: [
      {
        plan_id: 'plan-claude-pro',
        provider_id: 'claude_code',
        plan_name: 'Claude Pro',
        window_id: '5h',
        window_label: '5 小时窗口',
        used_percent: 42.5,
        resets_at_ms: 1_776_018_000_000,
        source: 'local_estimate',
      },
    ],
  };
}

describe('tokenvane-sync-worker SQLite Durable Object hub', () => {
  let db: DatabaseSync;
  let hub: SyncHubCore;
  let env: WorkerEnv;

  beforeEach(() => {
    db = new DatabaseSync(':memory:');
    hub = new SyncHubCore(new NodeSqliteAdapter(db));
    env = makeTestEnv(hub);
  });

  afterEach(() => {
    db.close();
  });

  it('serves /healthz publicly and enforces constant-time Bearer auth on /v1/sync/*', async () => {
    const healthRes = await handleWorkerFetch(
      new Request('https://sync.example.com/healthz'),
      env
    );
    expect(healthRes.status).toBe(200);
    const healthJson = (await healthRes.json()) as { ok: boolean; protocol_version: number };
    expect(healthJson.ok).toBe(true);
    expect(healthJson.protocol_version).toBe(1);

    const noAuth = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/pull?since=0'),
      env
    );
    expect(noAuth.status).toBe(401);

    const badAuth = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/pull?since=0', {
        headers: { Authorization: 'Bearer wrong-token' },
      }),
      env
    );
    expect(badAuth.status).toBe(401);
  });

  it('supports idempotent two-device plaintext push/pull and subscription snapshots', async () => {
    const authHeaders = {
      Authorization: 'Bearer super-secret-sync-token-2026',
      'Content-Type': 'application/json',
    };

    const pushA1 = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/push', {
        method: 'POST',
        headers: authHeaders,
        body: JSON.stringify({
          mode: 'plaintext',
          envelope: makeEnvelope('dev-a', 'Workstation A', 1000),
        }),
      }),
      env
    );
    expect(pushA1.status).toBe(200);

    // Re-push same PK(device_id, provider_id, model_id, bucket_start_ms) with updated token count
    const pushA2 = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/push', {
        method: 'POST',
        headers: authHeaders,
        body: JSON.stringify({
          mode: 'plaintext',
          envelope: makeEnvelope('dev-a', 'Workstation A', 1800),
        }),
      }),
      env
    );
    expect(pushA2.status).toBe(200);

    // Device B pulls excluding itself
    const pullForB = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/pull?since=0&exclude_device=dev-b', {
        headers: { Authorization: 'Bearer super-secret-sync-token-2026' },
      }),
      env
    );
    expect(pullForB.status).toBe(200);
    const pullData = (await pullForB.json()) as SyncPullResponseV1;
    expect(pullData.items).toHaveLength(1);
    expect(pullData.items[0].mode).toBe('plaintext');
    expect(pullData.items[0].envelope?.hourly_deltas).toHaveLength(1);
    expect(pullData.items[0].envelope?.hourly_deltas[0].input_tokens).toBe(1800);
    expect(pullData.items[0].envelope?.subscription_snapshots).toHaveLength(1);

    // Device A pulls excluding itself -> 0 remote items
    const pullForA = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/pull?since=0&exclude_device=dev-a', {
        headers: { Authorization: 'Bearer super-secret-sync-token-2026' },
      }),
      env
    );
    const pullAData = (await pullForA.json()) as SyncPullResponseV1;
    expect(pullAData.items).toHaveLength(0);
  });

  it('stores and streams opaque E2EE envelopes without server inspection while preserving seq_ms AAD', async () => {
    const authHeaders = {
      Authorization: 'Bearer super-secret-sync-token-2026',
      'Content-Type': 'application/json',
    };

    const pushE2ee = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/push', {
        method: 'POST',
        headers: authHeaders,
        body: JSON.stringify({
          mode: 'e2ee',
          encrypted_envelope: {
            schema_version: 1,
            device_id: 'dev-a',
            seq_ms: 1_776_000_123_456,
            kdf_salt_hex: '00112233445566778899aabbccddeeff',
            nonce_hex: 'aabbccddeeff001122334455',
            ciphertext_b64: 'ZmFrZV9jaXBoZXJ0ZXh0X2Zvcl90ZXN0',
          },
        }),
      }),
      env
    );
    expect(pushE2ee.status).toBe(200);

    const pullRes = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/pull?since=0&exclude_device=dev-b', {
        headers: { Authorization: 'Bearer super-secret-sync-token-2026' },
      }),
      env
    );
    const pullData = (await pullRes.json()) as SyncPullResponseV1;
    expect(pullData.items).toHaveLength(1);
    expect(pullData.items[0].mode).toBe('e2ee');
    expect(pullData.items[0].encrypted_envelope?.seq_ms).toBe(1_776_000_123_456);
    expect(pullData.items[0].encrypted_envelope?.ciphertext_b64).toBe(
      'ZmFrZV9jaXBoZXJ0ZXh0X2Zvcl90ZXN0'
    );
  });

  it('rejects any push payload containing forbidden compliance fields (session_id, prompt, file_path)', async () => {
    const res = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/push', {
        method: 'POST',
        headers: {
          Authorization: 'Bearer super-secret-sync-token-2026',
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          mode: 'plaintext',
          envelope: {
            ...makeEnvelope('dev-a', 'Workstation A', 100),
            session_id: 'secret-session-123',
          },
        }),
      }),
      env
    );
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: string };
    expect(body.error).toBe('COMPLIANCE_VIOLATION');
  });

  it('broadcasts real-time SSE sync_update events when another device pushes', async () => {
    const sseRes = await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/events', {
        headers: { Authorization: 'Bearer super-secret-sync-token-2026' },
      }),
      env
    );
    expect(sseRes.status).toBe(200);
    expect(sseRes.headers.get('content-type')).toContain('text/event-stream');

    const reader = sseRes.body?.getReader();
    expect(reader).toBeDefined();
    const decoder = new TextDecoder();

    const firstChunk = await reader!.read();
    expect(decoder.decode(firstChunk.value)).toContain(': connected');

    await handleWorkerFetch(
      new Request('https://sync.example.com/v1/sync/push', {
        method: 'POST',
        headers: {
          Authorization: 'Bearer super-secret-sync-token-2026',
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          mode: 'plaintext',
          envelope: makeEnvelope('dev-a', 'Workstation A', 500),
        }),
      }),
      env
    );

    const updateChunk = await reader!.read();
    const updateText = decoder.decode(updateChunk.value);
    expect(updateText).toContain('event: sync_update');
    expect(updateText).toContain('"device_id":"dev-a"');

    await reader!.cancel();
    expect(hub.activeSseCount).toBe(0);
  });

  it('pre-compiled bundle/index.js serves /healthz and handles push/pull identically to TS source', async () => {
    const bundleUrl = new URL('../../bundle/index.js', import.meta.url).href;
    const bundleMod = (await import(bundleUrl)) as {
      default: {
        fetch: (request: Request, env: WorkerEnv, ctx: CfExecutionContext) => Promise<Response>;
      };
      SyncHubDO: new (
        state: CfDurableObjectState,
        env: WorkerEnv
      ) => {
        fetch: (request: Request) => Promise<Response>;
      };
    };

    const adapter = new NodeSqliteAdapter(db);
    const bundledDo = new bundleMod.SyncHubDO(
      {
        storage: {
          sql: {
            exec: <T extends Record<string, unknown>>(
              query: string,
              ...bindings: Array<string | number | null>
            ) => ({
              toArray: () => adapter.exec<T>(query, ...bindings),
            }),
          },
        },
      },
      {} as WorkerEnv
    );
    const bundledEnv: WorkerEnv = {
      SYNC_TOKEN: 'bundle-test-token',
      SYNC_HUB: {
        idFromName: (name: string) => ({ toString: () => name }),
        get: () => ({
          fetch: (input: RequestInfo | URL, init?: RequestInit) => {
            const req = input instanceof Request ? input : new Request(input, init);
            return bundledDo.fetch(req);
          },
        }),
      },
    };
    const dummyCtx: CfExecutionContext = {
      waitUntil: () => {},
      passThroughOnException: () => {},
    };

    const healthRes = await bundleMod.default.fetch(
      new Request('https://bundle.example.workers.dev/healthz'),
      bundledEnv,
      dummyCtx
    );
    expect(healthRes.status).toBe(200);
    const health = (await healthRes.json()) as { ok: boolean; service: string; protocol_version: number };
    expect(health).toEqual({ ok: true, service: 'tokenvane-sync-worker', protocol_version: 1 });

    const pushRes = await bundleMod.default.fetch(
      new Request('https://bundle.example.workers.dev/v1/sync/push', {
        method: 'POST',
        headers: {
          Authorization: 'Bearer bundle-test-token',
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          mode: 'plaintext',
          envelope: makeEnvelope('dev-bundle-a', 'Bundle PC', 2400),
        }),
      }),
      bundledEnv,
      dummyCtx
    );
    expect(pushRes.status).toBe(200);

    const pullRes = await bundleMod.default.fetch(
      new Request('https://bundle.example.workers.dev/v1/sync/pull?since=0&exclude_device=dev-bundle-b', {
        headers: { Authorization: 'Bearer bundle-test-token' },
      }),
      bundledEnv,
      dummyCtx
    );
    expect(pullRes.status).toBe(200);
    const pullBody = (await pullRes.json()) as SyncPullResponseV1;
    expect(pullBody.items).toHaveLength(1);
    expect(pullBody.items[0].envelope?.hourly_deltas[0].input_tokens).toBe(2400);
  });
});
