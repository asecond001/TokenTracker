# TokenVane Sync Worker

基于 Cloudflare Workers 和 SQLite Durable Objects (`SyncHubDO`) 的轻量多端同步服务，无需额外购买或配置外部数据库。

它只做一件事：在你的多台电脑和手机之间同步**脱敏后的小时级用量汇总**和**订阅窗口百分比**（或者开启 E2EE 后的 `AES-256-GCM` 密文）。如果请求里夹带了 `prompt`、`messages`、`session_id`、文件路径或 API Key，网关会直接拒收。

## 3 分钟部署指南

1. 在仓库根目录安装依赖：
   ```bash
   pnpm install
   ```
2. 登录 Cloudflare 账号：
   ```bash
   npx wrangler login
   ```
3. 设置你自己的同步鉴权密钥 `SYNC_TOKEN`：
   ```bash
   cd services/sync-worker
   npx wrangler secret put SYNC_TOKEN
   ```
4. 部署到 Cloudflare：
   ```bash
   npx wrangler deploy
   ```
5. 验证服务状态：
   ```bash
   curl https://<你的-worker-子域名>.workers.dev/healthz
   ```
6. 打开 TokenVane 桌面端 **偏好设置 -> 云端多端同步**，填入 Worker 地址和 `SYNC_TOKEN`。如需端到端加密，勾选 **E2EE** 并在各设备间使用相同的加密口令。
