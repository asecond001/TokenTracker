declare module 'node:sqlite' {
  export interface StatementSync {
    all(...params: Array<string | number | bigint | Uint8Array | null>): Array<Record<string, unknown>>;
    run(...params: Array<string | number | bigint | Uint8Array | null>): {
      changes: number | bigint;
      lastInsertRowid: number | bigint;
    };
    columns?(): Array<{ name: string }>;
  }

  export class DatabaseSync {
    constructor(location: string);
    exec(sql: string): void;
    prepare(sql: string): StatementSync;
    close(): void;
  }
}

interface CfSqlStorageCursor<T extends Record<string, unknown> = Record<string, unknown>> {
  toArray(): T[];
}

interface CfSqlStorage {
  exec<T extends Record<string, unknown> = Record<string, unknown>>(
    query: string,
    ...bindings: Array<string | number | null>
  ): CfSqlStorageCursor<T>;
}

interface CfDurableObjectStorage {
  sql: CfSqlStorage;
}

interface CfDurableObjectState {
  storage: CfDurableObjectStorage;
}

interface CfDurableObjectStub {
  fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
}

interface CfDurableObjectId {
  toString(): string;
}

interface CfDurableObjectNamespace {
  idFromName(name: string): CfDurableObjectId;
  get(id: CfDurableObjectId): CfDurableObjectStub;
}

interface CfExecutionContext {
  waitUntil(promise: Promise<unknown>): void;
  passThroughOnException(): void;
}
