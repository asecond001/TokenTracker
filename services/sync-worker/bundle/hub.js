const MAX_PUSH_BYTES = 1_048_576;
const MAX_E2EE_ENVELOPES_PER_DEVICE = 500;
const SSE_HEARTBEAT_MS = 25_000;
const FORBIDDEN_KEYS = new Set([
    'session_id',
    'prompt',
    'messages',
    'file_path',
    'source_path',
    'project_label',
    'api_key',
    'passphrase',
    'sync_token',
]);
const textEncoder = new TextEncoder();
export function jsonResponse(body, status = 200) {
    return new Response(JSON.stringify(body), {
        status,
        headers: {
            'Content-Type': 'application/json; charset=utf-8',
            'Cache-Control': 'no-store',
        },
    });
}
export function findForbiddenComplianceKey(value, depth = 0) {
    if (depth > 16 || value === null || typeof value !== 'object') {
        return null;
    }
    if (Array.isArray(value)) {
        for (const item of value) {
            const hit = findForbiddenComplianceKey(item, depth + 1);
            if (hit)
                return hit;
        }
        return null;
    }
    const record = value;
    for (const [key, nested] of Object.entries(record)) {
        const normalized = key.trim().toLowerCase();
        if (FORBIDDEN_KEYS.has(normalized)) {
            return normalized;
        }
        const hit = findForbiddenComplianceKey(nested, depth + 1);
        if (hit)
            return hit;
    }
    return null;
}
export async function verifyBearerToken(authHeader, expectedToken) {
    const cleanExpected = expectedToken?.trim() ?? '';
    if (!cleanExpected) {
        return {
            ok: false,
            status: 503,
            message: 'Worker SYNC_TOKEN secret is not configured',
        };
    }
    if (!authHeader) {
        return { ok: false, status: 401, message: 'Missing Authorization Bearer header' };
    }
    const match = /^Bearer\s+(.+)$/i.exec(authHeader.trim());
    const presented = match?.[1]?.trim() ?? '';
    if (!presented) {
        return { ok: false, status: 401, message: 'Invalid Authorization Bearer scheme' };
    }
    const [expectedDigest, presentedDigest] = await Promise.all([
        crypto.subtle.digest('SHA-256', textEncoder.encode(cleanExpected)),
        crypto.subtle.digest('SHA-256', textEncoder.encode(presented)),
    ]);
    const a = new Uint8Array(expectedDigest);
    const b = new Uint8Array(presentedDigest);
    let diff = a.length ^ b.length;
    for (let i = 0; i < Math.min(a.length, b.length); i++) {
        diff |= a[i] ^ b[i];
    }
    if (diff !== 0) {
        return { ok: false, status: 401, message: 'Invalid sync Bearer token' };
    }
    return { ok: true };
}
export class SyncHubCore {
    sql;
    initialized = false;
    subscribers = new Set();
    constructor(sql) {
        this.sql = sql;
    }
    ensureSchema() {
        if (this.initialized)
            return;
        this.sql.exec(`
      CREATE TABLE IF NOT EXISTS sync_meta (
        key TEXT PRIMARY KEY,
        value_text TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS devices (
        device_id TEXT PRIMARY KEY,
        device_display_name TEXT NOT NULL,
        mode TEXT NOT NULL,
        last_seq_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL
      );
      CREATE TABLE IF NOT EXISTS device_hourly_rollups (
        device_id TEXT NOT NULL,
        provider_id TEXT NOT NULL,
        model_id TEXT NOT NULL,
        bucket_start_ms INTEGER NOT NULL,
        request_count INTEGER NOT NULL,
        observable_step_count INTEGER NOT NULL,
        input_tokens INTEGER NOT NULL,
        output_tokens INTEGER NOT NULL,
        cache_read_tokens INTEGER NOT NULL,
        cache_write_tokens INTEGER NOT NULL,
        reasoning_tokens INTEGER NOT NULL,
        estimated_cost_micro_usd INTEGER NOT NULL,
        cache_saved_micro_usd INTEGER NOT NULL,
        total_duration_ms INTEGER NOT NULL,
        tps_sample_count INTEGER NOT NULL,
        tps_sum REAL NOT NULL,
        max_output_tps REAL NOT NULL,
        updated_at_ms INTEGER NOT NULL,
        server_seq_ms INTEGER NOT NULL,
        PRIMARY KEY (device_id, provider_id, model_id, bucket_start_ms)
      );
      CREATE INDEX IF NOT EXISTS idx_hub_rollups_seq
        ON device_hourly_rollups (server_seq_ms ASC, device_id);
      CREATE TABLE IF NOT EXISTS device_subscriptions (
        device_id TEXT NOT NULL,
        plan_id TEXT NOT NULL,
        window_id TEXT NOT NULL,
        provider_id TEXT NOT NULL,
        plan_name TEXT NOT NULL,
        window_label TEXT NOT NULL,
        used_percent REAL NOT NULL,
        resets_at_ms INTEGER,
        source TEXT NOT NULL,
        monthly_fee_micro_usd INTEGER NOT NULL DEFAULT 0,
        server_seq_ms INTEGER NOT NULL,
        PRIMARY KEY (device_id, plan_id, window_id)
      );
      CREATE TABLE IF NOT EXISTS device_payg_snapshots (
        device_id TEXT NOT NULL,
        account_id TEXT NOT NULL,
        provider_id TEXT NOT NULL,
        account_name TEXT NOT NULL,
        enabled INTEGER NOT NULL DEFAULT 1,
        currency TEXT,
        total_balance_micro INTEGER,
        granted_balance_micro INTEGER,
        topped_up_balance_micro INTEGER,
        remaining_manual_balance_micro_usd INTEGER,
        monthly_budget_micro_usd INTEGER,
        today_cost_micro_usd INTEGER NOT NULL DEFAULT 0,
        month_cost_micro_usd INTEGER NOT NULL DEFAULT 0,
        fetched_at_ms INTEGER,
        updated_at_ms INTEGER NOT NULL,
        server_seq_ms INTEGER NOT NULL,
        PRIMARY KEY (device_id, account_id)
      );
      CREATE TABLE IF NOT EXISTS e2ee_envelopes (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        device_id TEXT NOT NULL,
        server_seq_ms INTEGER NOT NULL,
        payload_json TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL
      );
      CREATE INDEX IF NOT EXISTS idx_hub_e2ee_seq
        ON e2ee_envelopes (server_seq_ms ASC, device_id);
    `);
        const subCols = this.sql.exec('PRAGMA table_info(device_subscriptions)');
        if (!subCols.some((c) => c.name.toLowerCase() === 'monthly_fee_micro_usd')) {
            this.sql.exec('ALTER TABLE device_subscriptions ADD COLUMN monthly_fee_micro_usd INTEGER NOT NULL DEFAULT 0');
        }
        this.initialized = true;
    }
    getLastServerSeqMs() {
        this.ensureSchema();
        const rows = this.sql.exec("SELECT value_text FROM sync_meta WHERE key = 'last_server_seq_ms'");
        if (rows.length === 0)
            return 0;
        const parsed = Number(rows[0].value_text);
        return Number.isFinite(parsed) ? parsed : 0;
    }
    allocateServerSeqMs() {
        const prev = this.getLastServerSeqMs();
        const next = Math.max(Date.now(), prev + 1);
        this.sql.exec(`INSERT INTO sync_meta (key, value_text) VALUES ('last_server_seq_ms', ?1)
       ON CONFLICT(key) DO UPDATE SET value_text = excluded.value_text`, String(next));
        return next;
    }
    broadcastSyncUpdate(event) {
        const frame = `event: sync_update\ndata: ${JSON.stringify(event)}\n\n`;
        for (const sub of this.subscribers) {
            try {
                sub.send(frame);
            }
            catch {
                this.subscribers.delete(sub);
            }
        }
    }
    get activeSseCount() {
        return this.subscribers.size;
    }
    push(payload) {
        this.ensureSchema();
        const nowMs = Date.now();
        if (payload.mode === 'plaintext') {
            const env = payload.envelope;
            if (!env || env.schema_version !== 1) {
                throw new HubValidationError(400, 'Plaintext push requires schema_version = 1 envelope');
            }
            const deviceId = env.device_id?.trim();
            if (!deviceId) {
                throw new HubValidationError(400, 'device_id cannot be empty');
            }
            const displayName = env.device_display_name?.trim() || deviceId;
            const serverSeqMs = this.allocateServerSeqMs();
            this.sql.exec(`INSERT INTO devices (device_id, device_display_name, mode, last_seq_ms, updated_at_ms)
         VALUES (?1, ?2, 'plaintext', ?3, ?4)
         ON CONFLICT(device_id) DO UPDATE SET
           device_display_name = excluded.device_display_name,
           mode = 'plaintext',
           last_seq_ms = excluded.last_seq_ms,
           updated_at_ms = excluded.updated_at_ms`, deviceId, displayName, serverSeqMs, nowMs);
            this.sql.exec('DELETE FROM e2ee_envelopes WHERE device_id = ?1', deviceId);
            for (const delta of env.hourly_deltas ?? []) {
                this.sql.exec(`INSERT INTO device_hourly_rollups (
             device_id, provider_id, model_id, bucket_start_ms,
             request_count, observable_step_count, input_tokens, output_tokens,
             cache_read_tokens, cache_write_tokens, reasoning_tokens,
             estimated_cost_micro_usd, cache_saved_micro_usd, total_duration_ms,
             tps_sample_count, tps_sum, max_output_tps, updated_at_ms, server_seq_ms
           ) VALUES (
             ?1, ?2, ?3, ?4,
             ?5, ?6, ?7, ?8,
             ?9, ?10, ?11,
             ?12, ?13, ?14,
             ?15, ?16, ?17, ?18, ?19
           )
           ON CONFLICT(device_id, provider_id, model_id, bucket_start_ms) DO UPDATE SET
             request_count = excluded.request_count,
             observable_step_count = excluded.observable_step_count,
             input_tokens = excluded.input_tokens,
             output_tokens = excluded.output_tokens,
             cache_read_tokens = excluded.cache_read_tokens,
             cache_write_tokens = excluded.cache_write_tokens,
             reasoning_tokens = excluded.reasoning_tokens,
             estimated_cost_micro_usd = excluded.estimated_cost_micro_usd,
             cache_saved_micro_usd = excluded.cache_saved_micro_usd,
             total_duration_ms = excluded.total_duration_ms,
             tps_sample_count = excluded.tps_sample_count,
             tps_sum = excluded.tps_sum,
             max_output_tps = excluded.max_output_tps,
             updated_at_ms = excluded.updated_at_ms,
             server_seq_ms = excluded.server_seq_ms`, deviceId, delta.provider_id, delta.model_id, delta.bucket_start_ms, delta.request_count, delta.observable_step_count, delta.input_tokens, delta.output_tokens, delta.cache_read_tokens, delta.cache_write_tokens, delta.reasoning_tokens, delta.estimated_cost_micro_usd, delta.cache_saved_micro_usd, delta.total_duration_ms, delta.tps_sample_count, delta.tps_sum, delta.max_output_tps, delta.updated_at_ms, serverSeqMs);
            }
            this.sql.exec('DELETE FROM device_subscriptions WHERE device_id = ?1', deviceId);
            for (const snap of env.subscription_snapshots ?? []) {
                const monthlyFee = typeof snap.monthly_fee_micro_usd === 'number' &&
                    Number.isFinite(snap.monthly_fee_micro_usd)
                    ? Math.round(snap.monthly_fee_micro_usd)
                    : 0;
                this.sql.exec(`INSERT INTO device_subscriptions (
             device_id, plan_id, window_id, provider_id, plan_name,
             window_label, used_percent, resets_at_ms, source, monthly_fee_micro_usd, server_seq_ms
           ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)`, deviceId, snap.plan_id, snap.window_id, snap.provider_id, snap.plan_name, snap.window_label, snap.used_percent, snap.resets_at_ms ?? null, snap.source, monthlyFee, serverSeqMs);
            }
            this.sql.exec('DELETE FROM device_payg_snapshots WHERE device_id = ?1', deviceId);
            for (const payg of env.payg_snapshots ?? []) {
                this.sql.exec(`INSERT INTO device_payg_snapshots (
             device_id, account_id, provider_id, account_name, enabled,
             currency, total_balance_micro, granted_balance_micro, topped_up_balance_micro,
             remaining_manual_balance_micro_usd, monthly_budget_micro_usd,
             today_cost_micro_usd, month_cost_micro_usd, fetched_at_ms, updated_at_ms, server_seq_ms
           ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)`, deviceId, payg.account_id, payg.provider_id, payg.account_name, payg.enabled ? 1 : 0, payg.currency ?? null, payg.total_balance_micro ?? null, payg.granted_balance_micro ?? null, payg.topped_up_balance_micro ?? null, payg.remaining_manual_balance_micro_usd ?? null, payg.monthly_budget_micro_usd ?? null, payg.today_cost_micro_usd ?? 0, payg.month_cost_micro_usd ?? 0, payg.fetched_at_ms ?? null, payg.updated_at_ms, serverSeqMs);
            }
            this.broadcastSyncUpdate({
                seq_ms: serverSeqMs,
                device_id: deviceId,
                mode: 'plaintext',
            });
            return {
                ok: true,
                device_id: deviceId,
                mode: 'plaintext',
                server_seq_ms: serverSeqMs,
                hourly_deltas_stored: env.hourly_deltas?.length ?? 0,
                subscriptions_stored: env.subscription_snapshots?.length ?? 0,
            };
        }
        if (payload.mode === 'e2ee') {
            const enc = payload.encrypted_envelope;
            if (!enc || enc.schema_version !== 1) {
                throw new HubValidationError(400, 'E2EE push requires schema_version = 1 encrypted_envelope');
            }
            const deviceId = enc.device_id?.trim();
            if (!deviceId || !enc.kdf_salt_hex?.trim() || !enc.nonce_hex?.trim() || !enc.ciphertext_b64?.trim()) {
                throw new HubValidationError(400, 'EncryptedSyncEnvelopeV1 has empty required fields');
            }
            const serverSeqMs = this.allocateServerSeqMs();
            this.sql.exec(`INSERT INTO devices (device_id, device_display_name, mode, last_seq_ms, updated_at_ms)
         VALUES (?1, ?1, 'e2ee', ?2, ?3)
         ON CONFLICT(device_id) DO UPDATE SET
           mode = 'e2ee',
           last_seq_ms = excluded.last_seq_ms,
           updated_at_ms = excluded.updated_at_ms`, deviceId, serverSeqMs, nowMs);
            this.sql.exec('DELETE FROM device_hourly_rollups WHERE device_id = ?1', deviceId);
            this.sql.exec('DELETE FROM device_subscriptions WHERE device_id = ?1', deviceId);
            this.sql.exec('DELETE FROM device_payg_snapshots WHERE device_id = ?1', deviceId);
            this.sql.exec(`INSERT INTO e2ee_envelopes (device_id, server_seq_ms, payload_json, created_at_ms)
         VALUES (?1, ?2, ?3, ?4)`, deviceId, serverSeqMs, JSON.stringify(enc), nowMs);
            this.sql.exec(`DELETE FROM e2ee_envelopes
         WHERE device_id = ?1 AND id NOT IN (
           SELECT id FROM e2ee_envelopes
           WHERE device_id = ?1
           ORDER BY server_seq_ms DESC, id DESC
           LIMIT ?2
         )`, deviceId, MAX_E2EE_ENVELOPES_PER_DEVICE);
            this.broadcastSyncUpdate({
                seq_ms: serverSeqMs,
                device_id: deviceId,
                mode: 'e2ee',
            });
            return {
                ok: true,
                device_id: deviceId,
                mode: 'e2ee',
                server_seq_ms: serverSeqMs,
                hourly_deltas_stored: 0,
                subscriptions_stored: 0,
            };
        }
        throw new HubValidationError(400, 'Unsupported sync push mode');
    }
    pull(sinceSeqMs, excludeDeviceId, limit = 100) {
        this.ensureSchema();
        const safeSince = Number.isFinite(sinceSeqMs) ? Math.max(0, Math.floor(sinceSeqMs)) : 0;
        const clampedLimit = Math.min(200, Math.max(1, Math.floor(limit)));
        const exclude = excludeDeviceId?.trim() || null;
        const maxSeqMs = this.getLastServerSeqMs();
        const items = [];
        const plaintextDevices = this.sql.exec(exclude
            ? `SELECT device_id, device_display_name, last_seq_ms
           FROM devices
           WHERE mode = 'plaintext' AND last_seq_ms > ?1 AND device_id != ?2
           ORDER BY last_seq_ms ASC`
            : `SELECT device_id, device_display_name, last_seq_ms
           FROM devices
           WHERE mode = 'plaintext' AND last_seq_ms > ?1
           ORDER BY last_seq_ms ASC`, ...(exclude ? [safeSince, exclude] : [safeSince]));
        for (const dev of plaintextDevices) {
            const hourlyRows = this.sql.exec(`SELECT provider_id, model_id, bucket_start_ms,
                request_count, observable_step_count, input_tokens, output_tokens,
                cache_read_tokens, cache_write_tokens, reasoning_tokens,
                estimated_cost_micro_usd, cache_saved_micro_usd, total_duration_ms,
                tps_sample_count, tps_sum, max_output_tps, updated_at_ms
         FROM device_hourly_rollups
         WHERE device_id = ?1 AND server_seq_ms > ?2
         ORDER BY bucket_start_ms ASC`, dev.device_id, safeSince);
            const subRows = this.sql.exec(`SELECT plan_id, provider_id, plan_name, window_id, window_label,
                used_percent, resets_at_ms, source, monthly_fee_micro_usd
         FROM device_subscriptions
         WHERE device_id = ?1
         ORDER BY provider_id ASC, window_id ASC`, dev.device_id);
            const paygRows = this.sql.exec(`SELECT account_id, provider_id, account_name, enabled,
                currency, total_balance_micro, granted_balance_micro, topped_up_balance_micro,
                remaining_manual_balance_micro_usd, monthly_budget_micro_usd,
                today_cost_micro_usd, month_cost_micro_usd, fetched_at_ms, updated_at_ms
         FROM device_payg_snapshots
         WHERE device_id = ?1
         ORDER BY provider_id ASC, account_id ASC`, dev.device_id);
            items.push({
                mode: 'plaintext',
                seq_ms: Number(dev.last_seq_ms),
                device_id: dev.device_id,
                envelope: {
                    schema_version: 1,
                    device_id: dev.device_id,
                    device_display_name: dev.device_display_name,
                    generated_at_ms: Number(dev.last_seq_ms),
                    since_watermark_ms: safeSince,
                    hourly_deltas: hourlyRows.map((r) => ({
                        provider_id: r.provider_id,
                        model_id: r.model_id,
                        bucket_start_ms: Number(r.bucket_start_ms),
                        request_count: Number(r.request_count),
                        observable_step_count: Number(r.observable_step_count),
                        input_tokens: Number(r.input_tokens),
                        output_tokens: Number(r.output_tokens),
                        cache_read_tokens: Number(r.cache_read_tokens),
                        cache_write_tokens: Number(r.cache_write_tokens),
                        reasoning_tokens: Number(r.reasoning_tokens),
                        estimated_cost_micro_usd: Number(r.estimated_cost_micro_usd),
                        cache_saved_micro_usd: Number(r.cache_saved_micro_usd),
                        total_duration_ms: Number(r.total_duration_ms),
                        tps_sample_count: Number(r.tps_sample_count),
                        tps_sum: Number(r.tps_sum),
                        max_output_tps: Number(r.max_output_tps),
                        updated_at_ms: Number(r.updated_at_ms),
                    })),
                    subscription_snapshots: subRows.map((s) => ({
                        plan_id: s.plan_id,
                        provider_id: s.provider_id,
                        plan_name: s.plan_name,
                        window_id: s.window_id,
                        window_label: s.window_label,
                        used_percent: Number(s.used_percent),
                        resets_at_ms: s.resets_at_ms === null ? null : Number(s.resets_at_ms),
                        source: s.source,
                        monthly_fee_micro_usd: Number(s.monthly_fee_micro_usd ?? 0),
                    })),
                    payg_snapshots: paygRows.map((p) => ({
                        account_id: p.account_id,
                        provider_id: p.provider_id,
                        account_name: p.account_name,
                        enabled: Number(p.enabled) !== 0,
                        currency: p.currency,
                        total_balance_micro: p.total_balance_micro === null ? null : Number(p.total_balance_micro),
                        granted_balance_micro: p.granted_balance_micro === null ? null : Number(p.granted_balance_micro),
                        topped_up_balance_micro: p.topped_up_balance_micro === null ? null : Number(p.topped_up_balance_micro),
                        remaining_manual_balance_micro_usd: p.remaining_manual_balance_micro_usd === null
                            ? null
                            : Number(p.remaining_manual_balance_micro_usd),
                        monthly_budget_micro_usd: p.monthly_budget_micro_usd === null ? null : Number(p.monthly_budget_micro_usd),
                        today_cost_micro_usd: Number(p.today_cost_micro_usd ?? 0),
                        month_cost_micro_usd: Number(p.month_cost_micro_usd ?? 0),
                        fetched_at_ms: p.fetched_at_ms === null ? null : Number(p.fetched_at_ms),
                        updated_at_ms: Number(p.updated_at_ms),
                    })),
                },
                encrypted_envelope: null,
            });
        }
        const e2eeRows = this.sql.exec(exclude
            ? `SELECT device_id, server_seq_ms, payload_json
           FROM e2ee_envelopes
           WHERE server_seq_ms > ?1 AND device_id != ?2
           ORDER BY server_seq_ms ASC
           LIMIT ?3`
            : `SELECT device_id, server_seq_ms, payload_json
           FROM e2ee_envelopes
           WHERE server_seq_ms > ?1
           ORDER BY server_seq_ms ASC
           LIMIT ?2`, ...(exclude
            ? [safeSince, exclude, clampedLimit + 1]
            : [safeSince, clampedLimit + 1]));
        for (const row of e2eeRows) {
            const parsed = JSON.parse(row.payload_json);
            items.push({
                mode: 'e2ee',
                seq_ms: Number(row.server_seq_ms),
                device_id: row.device_id,
                envelope: null,
                encrypted_envelope: parsed,
            });
        }
        items.sort((a, b) => a.seq_ms - b.seq_ms);
        const hasMore = items.length > clampedLimit;
        const page = items.slice(0, clampedLimit);
        const nextSinceMs = hasMore
            ? page[page.length - 1].seq_ms
            : Math.max(safeSince, maxSeqMs);
        return {
            ok: true,
            server_seq_ms: maxSeqMs,
            next_since_ms: nextSinceMs,
            has_more: hasMore,
            items: page,
        };
    }
    createSseResponse(heartbeatMs = SSE_HEARTBEAT_MS) {
        let sub = null;
        let timer = null;
        const subscribers = this.subscribers;
        const cleanup = () => {
            if (timer !== null) {
                clearInterval(timer);
                timer = null;
            }
            if (sub !== null) {
                subscribers.delete(sub);
                sub = null;
            }
        };
        const stream = new ReadableStream({
            start(controller) {
                const send = (frame) => {
                    controller.enqueue(textEncoder.encode(frame));
                };
                const close = () => {
                    cleanup();
                    try {
                        controller.close();
                    }
                    catch {
                    }
                };
                sub = { send, close };
                subscribers.add(sub);
                send(': connected\n\n');
                timer = setInterval(() => {
                    try {
                        send(': keepalive\n\n');
                    }
                    catch {
                        cleanup();
                    }
                }, heartbeatMs);
            },
            cancel() {
                cleanup();
            },
        });
        return new Response(stream, {
            status: 200,
            headers: {
                'Content-Type': 'text/event-stream; charset=utf-8',
                'Cache-Control': 'no-cache, no-transform',
                Connection: 'keep-alive',
            },
        });
    }
    async handleHubRequest(request) {
        const url = new URL(request.url);
        if (request.method === 'POST' && url.pathname === '/v1/sync/push') {
            const contentLengthHeader = request.headers.get('content-length');
            if (contentLengthHeader) {
                const declared = Number(contentLengthHeader);
                if (Number.isFinite(declared) && declared > MAX_PUSH_BYTES) {
                    return jsonResponse({ ok: false, error: 'PAYLOAD_TOO_LARGE', message: 'Sync payload exceeds 1 MiB limit' }, 413);
                }
            }
            const rawText = await request.text();
            if (textEncoder.encode(rawText).byteLength > MAX_PUSH_BYTES) {
                return jsonResponse({ ok: false, error: 'PAYLOAD_TOO_LARGE', message: 'Sync payload exceeds 1 MiB limit' }, 413);
            }
            let parsed;
            try {
                parsed = JSON.parse(rawText);
            }
            catch {
                return jsonResponse({ ok: false, error: 'INVALID_JSON', message: 'Malformed JSON body' }, 400);
            }
            const forbiddenKey = findForbiddenComplianceKey(parsed);
            if (forbiddenKey) {
                return jsonResponse({
                    ok: false,
                    error: 'COMPLIANCE_VIOLATION',
                    message: `Forbidden sensitive field "${forbiddenKey}" rejected from cloud sync payload`,
                }, 400);
            }
            try {
                const result = this.push(parsed);
                return jsonResponse(result, 200);
            }
            catch (err) {
                if (err instanceof HubValidationError) {
                    return jsonResponse({ ok: false, error: 'VALIDATION_ERROR', message: err.message }, err.status);
                }
                const message = err instanceof Error ? err.message : 'Sync push failed';
                return jsonResponse({ ok: false, error: 'INTERNAL_ERROR', message }, 500);
            }
        }
        if (request.method === 'GET' && url.pathname === '/v1/sync/pull') {
            const sinceRaw = url.searchParams.get('since') ?? '0';
            const since = Number(sinceRaw);
            const excludeDevice = url.searchParams.get('exclude_device');
            const limitRaw = Number(url.searchParams.get('limit') ?? '100');
            const result = this.pull(since, excludeDevice, Number.isFinite(limitRaw) ? limitRaw : 100);
            return jsonResponse(result, 200);
        }
        if (request.method === 'GET' && url.pathname === '/v1/sync/events') {
            return this.createSseResponse();
        }
        return jsonResponse({ ok: false, error: 'NOT_FOUND', message: 'Route not found' }, 404);
    }
}
export class HubValidationError extends Error {
    status;
    constructor(status, message) {
        super(message);
        this.status = status;
    }
}
