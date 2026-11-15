# TokenTracker

面向开发者的**去卡片化独立桌面客户端与多端协同 AI Token 用量监测工具**。

## 核心特性

- **类 Linear 扁平桌面客户端体验**：左侧紧凑导航栏 + 右侧高密度扁平工作区 + 系统托盘常驻，以留白、`1px` 分割线、`tabular-nums` 等宽数字与全宽图表替代冗余卡片堆砌。
- **多 Provider 统一且合规采集**：首期覆盖 Claude Code、Codex 与 Antigravity，后续扩展至 Kimi Code、Workbuddy、Deepseek Harness、Qcode、Minimax Code 等；严格限定通过本地公开日志只读解析、本地 `127.0.0.1` OTLP 与系统凭据库安全存储的官方公开 API 获取数据。
- **全维度用量与效率洞察**：涵盖实时概览、多粒度趋势分析、月度热力图与订阅回本率账本、分模型统计与 API 价格换算、滚动窗口配额预警以及生成速度（TPS/延迟）监测。
- **面向多端原生演进**：以平台无关的 `tokentracker-core`（Rust）为底座，桌面端采用 `Tauri v2 + React + TypeScript + Tailwind CSS`，后续通过 `Cloudflare Worker (Durable Objects + SSE)` 同步，支撑 `macOS` 与纯原生 `iOS (SwiftUI)` / `Android (Jetpack Compose)` 客户端协同查看。

