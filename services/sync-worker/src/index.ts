import {
  jsonResponse,
  SyncHubCore,
  verifyBearerToken,
  type SqlAdapter,
} from './hub.js';

export interface WorkerEnv {
  SYNC_TOKEN?: string;
  SYNC_HUB: CfDurableObjectNamespace;
}

class CfSqlStorageAdapter implements SqlAdapter {
  private readonly sql: CfSqlStorage;

  constructor(sql: CfSqlStorage) {
    this.sql = sql;
  }

  exec<T extends Record<string, unknown> = Record<string, unknown>>(
    query: string,
    ...bindings: Array<string | number | null>
  ): T[] {
    return this.sql.exec<T>(query, ...bindings).toArray();
  }
}

export class SyncHubDO {
  private readonly hub: SyncHubCore;

  constructor(state: CfDurableObjectState, _env: WorkerEnv) {
    this.hub = new SyncHubCore(new CfSqlStorageAdapter(state.storage.sql));
  }

  async fetch(request: Request): Promise<Response> {
    return this.hub.handleHubRequest(request);
  }
}

export async function handleWorkerFetch(
  request: Request,
  env: WorkerEnv
): Promise<Response> {
  const url = new URL(request.url);

  if (request.method === 'GET' && url.pathname === '/healthz') {
    return jsonResponse(
      {
        ok: true,
        service: 'tokentracker-sync-worker',
        protocol_version: 1,
      },
      200
    );
  }

  if (url.pathname.startsWith('/v1/sync/')) {
    const auth = await verifyBearerToken(
      request.headers.get('Authorization'),
      env.SYNC_TOKEN
    );
    if (!auth.ok) {
      return jsonResponse(
        {
          ok: false,
          error: auth.status === 503 ? 'SERVER_MISCONFIGURED' : 'UNAUTHORIZED',
          message: auth.message,
        },
        auth.status
      );
    }

    const stub = env.SYNC_HUB.get(env.SYNC_HUB.idFromName('global-sync-hub'));
    return stub.fetch(request);
  }

  return jsonResponse({ ok: false, error: 'NOT_FOUND', message: 'Not found' }, 404);
}

export default {
  fetch(request: Request, env: WorkerEnv, _ctx: CfExecutionContext): Promise<Response> {
    return handleWorkerFetch(request, env);
  },
};
