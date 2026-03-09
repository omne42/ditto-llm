# Provider Runtime Rollout

本页记录 `ditto-llm` 在 provider runtime 落地阶段的执行顺序与收口结果。
它只讨论仓库内的 L0/L1 runtime，不讨论独立 L2 平台能力。

## 1) Snapshot（2026-03-08）

来自 `cargo run --all-features --bin ditto-catalog-dashboard` 的当前结论：

- 所有 reference catalog provider 都已达到 reference/runtime 对齐：
  - `anthropic`
  - `bailian`
  - `deepseek`
  - `doubao`
  - `google`
  - `hunyuan`
  - `kimi`
  - `minimax`
  - `openai`
  - `openrouter`
  - `qianfan`
  - `xai`
  - `zhipu`
- provider 级 runtime gap 已清零；`CATALOG_COMPLETENESS.md` 不再存在 planned/missing provider capability。
- 仍未公共化为稳定 `cap-*` 的 runtime capability：
  - `context.cache`
  - `video.generation`
  - `image.translation`
  - `image.question`
  - `audio.voice_clone`
  - `audio.voice_design`
  - `music.generation`
  - `3d.generation`
  - `ocr`
  - `classification_or_extraction`

说明：`openai-compatible` 仍然是一个有意保留的泛化 runtime provider，而不是 reference catalog provider；它不属于这里的“补齐 catalog”范围。

## 2) 为什么保留这份文档

`CATALOG_COMPLETENESS.md` 负责当前事实盘点。

这份文档保留的是 rollout 过程中的约束与后续动作：

- 哪些能力先抽象、后接 provider
- 哪些工作已经收口，避免以后回退成 scattered `if/else`
- provider-level gap 清零之后，下一步应该转向哪些 public API / feature pack 决策

## 3) 执行规则

1. 先抽象共享 capability，再接 provider。
   不要先在 provider client 里堆条件分支，再事后抽象。
2. public `cap-*` 只为跨 provider 的稳定能力建模。
   单 provider 或短期实验能力先保留在 runtime registry，不要急着公开成长期 API。
3. 每个新增 capability 都必须同时经过五层：
   - catalog binding
   - typed builder / trait
   - gateway exposure / capability gating
   - contract tests
   - dashboard / docs
4. `CATALOG_COMPLETENESS.md` 是 runtime truth；`PROVIDERS.md` 只展示稳定 public surface。
   两者不一致时，先修 truth，再修 public docs。

## 4) Workstream 收口结果

### A. `context.cache` 共享抽象（已完成）

目标 provider：`deepseek`、`minimax`

收口结果：

- [x] dashboard 中 `deepseek` 与 `minimax` 的 `missing` 已归零
- [x] 已新增 typed `ContextCacheModel` / `ContextCacheProfile`
- [x] 已补 builder 与 provider tests，runtime 可显式解析 context cache profile
- [ ] `cap-context-cache` 的公共化仍是后续 API 设计工作，不属于 provider gap

### B. `video.generation` 共享抽象（已完成：OpenAI + Google）

目标 provider：`openai`、`google`

收口结果：

- [x] 在 L0 新增 typed `VideoGenerationModel`
- [x] OpenAI native `/videos` 资源 API 已接入，并补齐 builder、route、provider tests
- [x] Google native `predictLongRunning` 已接入，并补齐 builder、route、provider tests
- [x] dashboard 中 `openai` 与 `google` 的 `video.generation` planned/missing 已清零
- [ ] `cap-video-generation` 的公共化仍待后续 feature/API 决策

备注：Google 的 `VideoGenerationModel` 当前以长任务创建/查询/下载为主，`list/delete/remix` 维持显式 unsupported，不假装存在资源型 surface。

### C. Google `image.generation` 与 `realtime`（已完成）

目标 provider：`google`

收口结果：

- [x] 已新增 Google native image builder，处理 `POST ...:predict`
- [x] 已新增 Google live realtime adapter，输出 typed websocket session + `setup_payload`
- [x] gateway capability gating 与 builder 都已接通
- [x] dashboard 中 `google` 的 `image.generation` 与 `realtime` planned 已清零
- [x] `PROVIDERS.md` 与 feature docs 已从 planned 改成 implemented

### D. public capability packs 的公共化策略（下一阶段）

这已经不是“补齐 provider runtime”，而是“把已存在的 runtime 能力升级成稳定公开能力”的设计工作。

优先级建议：

1. `cap-context-cache`
2. `cap-video-generation`
3. `cap-ocr`
4. 其它 provider-specific niche capability 继续保留在 runtime registry

公共化前提：

- 至少两个 provider 已实现，或者一个默认重要 provider 已稳定
- 先有 typed API，而不是只存在 runtime registry 与原始 JSON
- 升格后必须同步更新：`Cargo.toml`、`PROVIDERS.md`、`docs/src/concepts/features.md`、dashboard、默认 CI feature matrix

## 5) 后续顺序

1. 转向 public capability packs 的命名与暴露策略，避免 runtime 已有能力继续长期停留在“内部能力”状态。
2. 保持 watch mode；只有当 reference docs 变化、dashboard 再次出现 planned/missing 时才重新开启 provider rollout。

## 6) Stop Gate

每次继续推进 capability 公共化或新增 provider runtime 时，至少重新通过下面这组检查：

```bash
cargo run --all-features --bin ditto-catalog-dashboard
cargo run --all-features --bin ditto-catalog-dashboard -- --check
cargo run --bin ditto-llms-txt
cargo run --bin ditto-llms-txt -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

并且要求：

- `CATALOG_COMPLETENESS.md` 与 dashboard 结果保持一致
- `PROVIDERS.md` / feature docs 不再把已实现能力写成 planned
- 新 capability 没有绕过 typed layer 直接落到裸 JSON merge
