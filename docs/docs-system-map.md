# ditto-llm Docs System

## 入口分工

- `README.md`
  - 对外概览、功能面和快速开始。
- `AGENTS.md`
  - 给执行者的短地图。
- `docs/README.md`
  - 版本化文档入口，负责把关键事实路径串起来。
- `docs/`
  - 版本化事实来源。

## 目录职责

- `docs/architecture/`
  - `system-boundaries.md`：本仓负责什么、复用什么、哪些能力继续留在本仓。
  - `source-layout.md`：源码树与目录职责。
- `docs/src/`
  - `mdBook` 源文档，是 SDK / Gateway / Clients / Migration 手册的主事实来源。
- `docs/book/`
  - `mdBook` 生成站点，不是事实来源。
- `docs/tmp/`
  - 临时调研和工作笔记，不是规范来源。
- `contracts/`
  - 对外契约产物。

## 新鲜度规则

- provider/runtime/gateway 边界变化时，同时更新 `system-boundaries.md` 和相关 `docs/src/*.md`。
- 目录职责变化时，更新 `source-layout.md`。
- `mdBook` 导航变化时，更新 `docs/src/SUMMARY.md`。
- 不直接维护 `docs/book/`。
- 当共享基建复用状态变化时，更新边界文档，明确哪些已经落到 `omne_foundation` / `omne-runtime`，哪些仍然是本仓私有实现。
- `scripts/check-docs-system.sh` 机械检查根入口与关键 docs 骨架。

关键事实入口：

- `docs/architecture/system-boundaries.md`
- `docs/architecture/source-layout.md`
- `docs/src/`
