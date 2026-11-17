import { jsonResponse, SyncHubCore, verifyBearerToken, } from './hub.js';
class CfSqlStorageAdapter {
    sql;
    constructor(sql) {
        this.sql = sql;
    }
    exec(query, ...bindings) {
        return this.sql.exec(query, ...bindings).toArray();
    }
}
export class SyncHubDO {
    hub;
    constructor(state, _env) {
        this.hub = new SyncHubCore(new CfSqlStorageAdapter(state.storage.sql));
    }
    async fetch(request) {
        return this.hub.handleHubRequest(request);
    }
}
export async function handleWorkerFetch(request, env) {
    const url = new URL(request.url);
    if (request.method === 'GET' && url.pathname === '/healthz') {
        return jsonResponse({
            ok: true,
            service: 'tokenvane-sync-worker',
            protocol_version: 1,
        }, 200);
    }
    if (url.pathname.startsWith('/v1/sync/')) {
        const auth = await verifyBearerToken(request.headers.get('Authorization'), env.SYNC_TOKEN);
        if (!auth.ok) {
            return jsonResponse({
                ok: false,
                error: auth.status === 503 ? 'SERVER_MISCONFIGURED' : 'UNAUTHORIZED',
                message: auth.message,
            }, auth.status);
        }
        const stub = env.SYNC_HUB.get(env.SYNC_HUB.idFromName('global-sync-hub'));
        return stub.fetch(request);
    }
    return jsonResponse({ ok: false, error: 'NOT_FOUND', message: 'Not found' }, 404);
}
export default {
    fetch(request, env, _ctx) {
        return handleWorkerFetch(request, env);
    },
};
