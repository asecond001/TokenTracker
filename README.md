# TokenTracker

面向开发者的**去卡片化独立桌面客户端与多端协同 AI Token 用量监测工具**（当前已完成 **Phase 1 v0.1.2 交互收束、图表渲染修复与价格汇率自动同步专版**，基于 **SQLite Schema v3** 与 **32 个强类型 Tauri IPC 接口**）。

## 核心特性

- **类 Linear 扁平桌面客户端与精简交互层级**：`38px` 一体化标题栏 + `208px` 左侧紧凑导航栏 + 右侧高密度扁平工作区 + 系统托盘常驻与快捷浮层（`?view=tray`）；将 `<TimeRangeControl />` 精准内聚至真正需要时间筛选的统计视图（概览、趋势分析、速度监测、模型与价格），支持深/浅/跟随系统零延迟同步及纯色 / 微光渐变 / 本地壁纸背景。
- **多 Provider 统一且合规采集**：首期覆盖 **Claude Code**、**Codex** 与 **Antigravity**，严格限定通过三类合法通道获取数据：
  1. 共享只读锁增量尾随本地公开会话 JSONL 与统计文件（`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`，不阻塞宿主工具写入）；
  2. 可选本地回环 `127.0.0.1` OTLP/HTTP 接收器（`/v1/metrics`、`/v1/logs`、`/healthz`）；
  3. 托管在操作系统原生凭据库（Windows Credential Manager）中的官方公开 Usage API 对账密钥。
- **汇率与模型单价全自动免配置同步**：基于公开免鉴权的 `Frankfurter` / `Open Exchange Rates` 实时汇率（12h TTL）与开源 `models.dev` / `LiteLLM` 模型定价仓库（24h TTL）在启动及每 6 小时静默更新并按需重算；支持离线基线无感兜底、用户自定义单价覆盖优先（`is_user_override = 1`）以及一键锁定手动汇率。
- **全维度与零歧义可视化洞察**：
  - 概览堆叠面积图自动剔除 `0 Token` 序列、消除跨序列描边蓝框伪影并解耦 Token 类型与 Provider 品牌调色板；
  - 月度 365 天活跃热力图与按日账本统一对齐本地时区小时桶，按四分位离散色阶（含仅可观测活跃日）精确分层上色，并支持 CSV/JSON 导出；
  - 零预设诚实订阅体系（7 套模板按需录入 + 未绑定套餐官方限额快照）与 Provider / `Model × Provider` 生成速度与延迟分布对比。
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
- MSI 安装包：`target/release/bundle/msi/TokenTracker_0.1.2_x64_en-US.msi`
- NSIS 安装包：`target/release/bundle/nsis/TokenTracker_0.1.2_x64-setup.exe`

