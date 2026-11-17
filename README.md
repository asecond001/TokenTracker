# TokenVane

一个用来算清 AI 编程工具到底花了多少 Token、合多少钱、离订阅限流还有多远的桌面端与 iOS 监测工具。

平时同时开着 Claude Code、Codex、Antigravity、Kimi 或 DeepSeek 写代码时，账单和额度往往散落在各个终端和网页后台里。TokenVane 把它们汇总到一个干净、像 Linear 一样克制的界面里，不搞满屏花哨的圆角卡片，也不碰任何中间人抓包或窃取 Cookie 的灰产手段。

---

## 它能做什么

- **安静、高密度的桌面工作台（Windows）**
  - 提供概览、趋势、月度账单（含 365 天热力图）、模型单价、订阅配额与生成速度拆解。
  - 常驻系统托盘，点一下就能看到今天的实时花费、各工具占比、最近 15 分钟活跃的编码会话，以及哪个订阅窗口快到限额了。
  - 支持深浅色跟随系统、`USD` / `CNY` 实时换算、多台电脑汇总或单机筛选。

- **开箱支持 8 款主流工具 + 自定义日志接入**
  - 内置 **Claude Code**、**Codex**、**Antigravity**、**Kimi Code**、**Workbuddy**、**DeepSeek Harness (`dsh`)**、**QCode** 与 **MiniMax Code**。
  - 如果你在用其他自建网关或 CLI，可以直接在界面里配置兼容 OpenAI / Anthropic 格式或 JSON 指针的本地日志规则，并用单行样本实时测试。

- **克制、合规的数据获取方式**
  - **本地只读日志**：用共享读锁增量尾随各工具正常落盘的 JSONL 日志，不抢锁、不改文件。
  - **本地 OTLP 接收**：可选监听 `127.0.0.1` 回环端口，接收工具按官方文档导出的 OpenTelemetry 遥测。
  - **官方公开接口**：如果你主动配置了官方 API Key（保存在系统凭据库中），可以定期查询余额与官方配额窗口，并自动将识别到的套餐加入订阅监控。

- **3 分钟自托管多端同步 + 纯原生 iOS 客户端**
  - 通过自托管的 Cloudflare Worker + Durable Objects 同步多台电脑的用量汇总，不需要额外部署数据库。
  - **绝对不上传**原始 Prompt、代码内容、会话 ID、本地路径或 API Key；支持开启 `AES-256-GCM` 端到端加密（E2EE），让服务端只存密文。
  - **iOS / iPadOS 原生客户端（`apps/ios`）**：纯 `SwiftUI` + `Swift Charts` 编写，通过 `UniFFI` 直接复用 Rust 核心计费与解密逻辑。支持主屏/锁屏小组件（`WidgetKit`）和**灵动岛实时盯盘（`Live Activities`）**，高强度写代码时在手机锁屏上就能看到 5h / 7d 窗口倒计时。

---

## 快速开始（本地开发与构建）

### 环境准备

- Rust (`stable-x86_64-pc-windows-msvc`) 与 MSVC Build Tools
- Node.js 20+ 与 `pnpm`
- Windows WebView2 Runtime

### 常用命令

```powershell
# 安装前端与工作区依赖
pnpm install

# Rust 静态检查与测试
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline

# 前端与 Worker 类型检查及单元测试
pnpm typecheck
pnpm test

# 构建前端与桌面端安装包 (.msi / NSIS .exe)
pnpm build
pnpm desktop:build

# 运行 iOS 工程与跨语言绑定本地校验（无需 Mac）
pnpm run ios:check:local

# 运行 100 万行 JSONL 冷启动与增量续读压测
cargo run --release --offline -p tokvane-collectors --example bench_million_lines
```

打包完成后，桌面端产物位于：
- 可执行文件：`target/release/tokvane-desktop.exe`
- MSI 安装包：`target/release/bundle/msi/TokenVane_0.2.3_x64_en-US.msi`
- NSIS 安装包：`target/release/bundle/nsis/TokenVane_0.2.3_x64-setup.exe`

---

## 3 分钟自托管云端同步（Cloudflare Worker）

如果你想把多台电脑的用量汇总到一起，或者在手机上查看实时配额，可以部署仓库自带的 `services/sync-worker`：

1. 登录 Cloudflare 并设置一个你自己定好的同步口令 `SYNC_TOKEN`：
   ```powershell
   cd services/sync-worker
   npx wrangler login
   npx wrangler secret put SYNC_TOKEN
   ```
2. 一键部署 Worker：
   ```powershell
   npx wrangler deploy
   curl https://<你的-worker-子域名>.workers.dev/healthz
   ```
3. 打开桌面端 **偏好设置 -> 云端多端同步**，填入 Worker 地址和 `SYNC_TOKEN`。建议顺手打开 **端到端加密（E2EE）** 并设一个加密口令。iOS 端可以直接扫码或粘贴配对信息接入。

---

## 自动更新与发布签名

推送到 `main` 或提交 PR 时，`.github/workflows/ci.yml` 会运行前端、Sync Worker 与 Rust 三组检查；推送 `v*` 标签时，`.github/workflows/release-desktop.yml` 复用同一门禁，打包签名安装包并生成 `latest.json` 更新清单。

如果要维护自己的发布源：
1. 运行 `pnpm tauri signer generate` 生成 Minisign 密钥对。
2. 把私钥存进 GitHub 仓库 Secrets 的 `TAURI_SIGNING_PRIVATE_KEY`（以及可选的 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）。
3. 把公钥填到 `apps/desktop/src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。

