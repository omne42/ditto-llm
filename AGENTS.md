# ditto-llm AGENTS Map

这个文件只做导航。稳定事实写在 `README.md` 和 `docs/`。

## 先看哪里

- 外部概览：`README.md`
- 文档入口：`docs/README.md`
- 文档系统地图：`docs/docs-system-map.md`
- 系统边界：`docs/architecture/system-boundaries.md`
- 源码布局：`docs/architecture/source-layout.md`
- 产品手册入口：`docs/src/index.md`
- `mdBook` 导航：`docs/src/SUMMARY.md`
- 外部契约：`contracts/gateway-contract-v0.1.openapi.yaml`

## 修改规则

- `docs/src/` 是 `mdBook` 源事实；不要直接改 `docs/book/`。
- SDK、gateway、packages 或复用边界变化时，更新 `docs/architecture/system-boundaries.md` 和相关 `docs/src/*.md`。
- 目录职责变化时，更新 `docs/architecture/source-layout.md`。
- 基础设施复用状态变化时，明确写“已经使用什么”和“仍然留在本仓什么”，不要混写。

## 验证

- `./scripts/check-docs-system.sh`
- `cargo fmt --all`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `mdbook build docs`
