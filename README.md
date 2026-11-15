# TokenTracker

面向开发者的**去卡片化独立桌面客户端与多端协同 AI Token 用量监测工具**（当前已完成 **Phase 1 v0.1 Windows 桌面端 MVP**）。

## 核心特性

- **类 Linear 扁平桌面客户端体验**：`38px` 一体化标题栏 + `208px` 左侧紧凑导航栏 + 右侧高密度扁平工作区 + 系统托盘常驻与快捷浮层（`?view=tray`），以留白、`1px` 细分割线、`tabular-nums` 等宽数字与无边框全宽图表替代冗余卡片堆砌。
- **多 Provider 统一且合规采集**：首期覆盖 **Claude Code**、**Codex** 与 **Antigravity**，严格限定通过三类合法通道获取数据：
  1. 共享只读锁增量尾随本地公开会话 JSONL 与统计文件（`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`，不阻塞宿主工具写入）；
  2. 可选本地回环 `127.0.0.1` OTLP/HTTP 接收器（`/v1/metrics`、`/v1/logs`、`/healthz`）；
  3. 托管在操作系统原生凭据库（Windows Credential Manager）中的官方公开 Usage API 对账密钥。
- **全维度用量与效率洞察**：涵盖概览、多粒度多维趋势分析、月度 365 天热力图与订阅回本率账本（CSV/JSON 导出）、分模型统计与 `rust_decimal` 定点数 API 价格换算（支持 `USD`/`CNY` 切换与自定义覆盖）、5h/7d 滚动窗口配额预警以及生成速度（TPS/P50/P95/上下文长度衰减）监测。
- **面向多端原生演进**：平台无关核心库 `crates/tokentracker-core`（含 `SyncEnvelope v1` 脱敏同步契约）与桌面采集引擎 `crates/tokentracker-collectors` 物理隔离，为后续接入 `Cloudflare Worker (Durable Objects + SSE)` 与纯原生移动端预留清晰边界。

## 本地开发、测试与打包（Windows）

### 前置环境

- Rust 工具链 (`stable-x86_64-pc-windows-msvc`) 与 MSVC Build Tools
- Node.js 20+ 与 `pnpm`
- Windows WebView2 Runtime

### 常用命令

```powershell
# 1. 运行 Rust 单元测试与集成测试（core + collectors + desktop）
cargo test --workspace --offline

# 2. 运行 Rust Clippy 静态检查（零警告门禁）
cargo clippy --workspace --all-targets --offline -- -D warnings

# 3. 运行前端 TypeScript 类型检查、Vitest 单元测试与 Vite 生产构建
pnpm typecheck
pnpm test
pnpm build

# 4. 运行 100 万行 JSONL 冷启动与增量写入基准测试（Release）
cargo run --release --offline -p tokentracker-collectors --example bench_million_lines

# 5. 构建桌面安装包（.msi 与 NSIS .exe）
pnpm desktop:build
```

打包产物默认输出在：
- 主程序：`target/release/tokentracker-desktop.exe`
- MSI 安装包：`target/release/bundle/msi/TokenTracker_0.1.0_x64_en-US.msi`
- NSIS 安装包：`target/release/bundle/nsis/TokenTracker_0.1.0_x64-setup.exe`

