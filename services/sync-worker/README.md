# TokenTracker Self-Hosted Sync Worker (Cloudflare Workers + SQLite Durable Objects)

Zero-external-database synchronization hub for TokenTracker desktop clients.
Only desensitized `SyncEnvelopeV1` hourly rollups & subscription window percentages—or client-side `AES-256-GCM` `EncryptedSyncEnvelopeV1` ciphertexts—are persisted inside a single SQLite-backed Durable Object (`SyncHubDO`). Prompts, code, file paths, session IDs, and API keys are strictly rejected.

## 3-Minute Self-Hosting Guide

1. Install dependencies from the repository root:
   ```bash
   pnpm install
   ```
2. Authenticate Wrangler (one time):
   ```bash
   npx wrangler login
   ```
3. Generate and store a strong Bearer token secret (`SYNC_TOKEN`):
   ```bash
   cd services/sync-worker
   npx wrangler secret put SYNC_TOKEN
   ```
4. Deploy the Worker + Durable Object migration:
   ```bash
   npx wrangler deploy
   ```
5. Verify health endpoint:
   ```bash
   curl https://<your-worker-subdomain>.workers.dev/healthz
   ```
6. Open TokenTracker Desktop -> **偏好设置 -> 云端多端同步**, paste your Worker URL and `SYNC_TOKEN`, optionally enable **E2EE 端到端加密** with a shared passphrase across your devices, and click **保存同步凭据 -> 立即同步**.
