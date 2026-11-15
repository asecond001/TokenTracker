# TokenTracker

面向开发者的**去卡片化独立桌面客户端与多端协同 AI Token 用量监测工具**（当前已完成 **Phase 2 / v0.2.0 — 实时摘要升级、第二批 Provider 扩展、Cloudflare 多端同步与自动更新**，基于 **SQLite Schema v4** 与 **45 个强类型 Tauri IPC 接口**）。

## 核心特性

- **类 Linear 扁平桌面客户端与实时态势感知**：
  - `38px` 一体化标题栏 + `208px` 左侧导航栏 + 高密度扁平主画布，支持深/浅/跟随系统零延迟双窗口同步、纯色/微光渐变/托管壁纸背景、`USD`/`CNY` 实时换算与「全部设备 / 单设备」即时切换。
  - `OverviewPage` 顶部与 `360×480` 系统托盘浮层（`?view=tray`）提供今日动态花费/Token 主读数、分段占比条、Top 3 模型拆解与本地 15 分钟窗口活跃（`<= 120s`）/ 近期 Session 列表，支持从托盘点击活跃会话一键跳转主工作台对应 Provider 过滤视图。
- **8 大主流 AI Provider 内置支持 + 声明式自定义接入层**：
  - 全量覆盖 **Claude Code**、**Codex**、**Antigravity**、**Kimi Code**、**Workbuddy**、**DeepSeek Harness (`dsh`)**（Prefix Cache 命中/未命中分离核算）、**QCode** 与 **MiniMax Code**，通过三类合规通道（共享只读文件锁尾随、`127.0.0.1` 回环 OTLP、Windows Credential Manager 托管官方公开 API）获取数据。
  - 提供 **自定义兼容规则引擎**（`openai_jsonl`、`anthropic_jsonl` 与 RFC 6901 JSON 指针 `declarative_jsonl`）及本地单行样本干跑校验；内置 Moonshot、DeepSeek、QCode、MiniMax 官方公开余额/额度查询（带 `60s` 冷却防限流）。
- **Cloudflare Worker 自托管实时同步（含可选 E2EE）**：
  - `services/sync-worker` 基于 Cloudflare Workers + SQLite-backed Durable Objects（`SyncHubDO`）实现零外部数据库云同步，提供 `POST /v1/sync/push`、`GET /v1/sync/pull` 与 `/v1/sync/events`（SSE + `25s` 心跳）。
  - 仅同步脱敏小时汇总与订阅百分比快照（网关层主动拦截含 `session_id` / `prompt` / `messages` / `file_path` / 密钥等字段的请求）；可选开启客户端 `PBKDF2-HMAC-SHA256`（`210,000` 次）+ `AES-256-GCM` 端到端加密，服务端仅存不可解密密文。
- **汇率/模型定价自动同步与零成本自更新**：
  - 定时免鉴权拉取 `Frankfurter` / `Open Exchange Rates` 汇率（12h TTL）与 `models.dev` / `LiteLLM` 模型单价目录（24h TTL）；集成 `tauri-plugin-updater` + GitHub Actions 发布签名清单（`latest.json`）实现静默检查与内联进度安装。

## Cloudflare Sync Worker 3 分钟自托管指南

1. 在仓库根目录安装依赖：
   ```powershell
   pnpm install
   ```
2. 登录 Cloudflare CLI 并设置强随机同步鉴权密钥 `SYNC_TOKEN`：
   ```powershell
   cd services/sync-worker
   npx wrangler login
   npx wrangler secret put SYNC_TOKEN
   ```
3. 部署 Worker 与 `SyncHubDO` Durable Object：
   ```powershell
   npx wrangler deploy
   curl https://<your-worker-subdomain>.workers.dev/healthz
   ```
4. 在 TokenTracker 客户端打开 **偏好设置 -> 云端多端同步（Cloudflare Worker）**：填入 Worker URL 与 `SYNC_TOKEN`，可选开启 **端到端加密（E2EE）** 并在多台电脑输入相同的同步口令，点击 **保存同步凭据 -> 立即同步**。

## GitHub Releases 自动更新与签名配置说明

- 桌面客户端通过 `.github/workflows/release-desktop.yml` 在推送 `v*` Tag 时自动执行前端/Rust 全量测试与 Clippy 门禁，并构建签名的 `.msi` / NSIS 安装包与 `latest.json`。
- 仓库维保者在发布前需通过 `pnpm tauri signer generate` 生成 Minisign 密钥对，将私钥写入 GitHub Repository Secrets 中的 `TAURI_SIGNING_PRIVATE_KEY`（及可选 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`），并将对应公钥填入 `apps/desktop/src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。

## 本地开发、测试与打包（Windows）

### 前置环境

- Rust 工具链 (`stable-x86_64-pc-windows-msvc`) 与 MSVC Build Tools
- Node.js 20+ 与 `pnpm`
- Windows WebView2 Runtime

### 常用命令

```powershell
# 1. 运行 Rust Clippy 静态检查（零警告门禁）
cargo clippy --workspace --all-targets --offline -- -D warnings

# 2. 运行 Rust 单元测试与集成测试（core + collectors + desktop）
cargo test --workspace --offline

# 3. 运行工作区 TypeScript 类型检查（apps/desktop + services/sync-worker）
pnpm typecheck

# 4. 运行前端与 Sync Worker 单元测试（Vitest）
pnpm test

# 5. 构建前端生产包
pnpm build

# 6. 运行 100 万行 JSONL 冷启动与增量写入基准测试（Release）
cargo run --release --offline -p tokentracker-collectors --example bench_million_lines

# 7. 构建桌面安装包（.msi 与 NSIS .exe）
pnpm desktop:build
```

打包产物默认输出在：
- 主程序：`target/release/tokentracker-desktop.exe`
- MSI 安装包：`target/release/bundle/msi/TokenTracker_0.2.0_x64_en-US.msi`
- NSIS 安装包：`target/release/bundle/nsis/TokenTracker_0.2.0_x64-setup.exe`

