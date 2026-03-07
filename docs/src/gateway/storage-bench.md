# 存储基准（audit + reap）

Ditto 提供了一个轻量基准工具：`ditto-store-bench`。

用途：

- 对比不同 store 的审计写入吞吐（`append_audit_log`）
- 对比审计清理耗时（`reap_audit_logs_before`）
- 对比 stale reservation 回收耗时（`reap_stale_*_reservations`）

## 运行方式

仅 sqlite：

```bash
cargo run --features "gateway gateway-store-sqlite" --bin ditto-store-bench -- \
  --sqlite /tmp/ditto-bench.sqlite \
  --audit-ops 5000 \
  --reap-ops 2000
```

sqlite + postgres + mysql：

```bash
cargo run --features "gateway gateway-store-sqlite gateway-store-postgres gateway-store-mysql" \
  --bin ditto-store-bench -- \
  --sqlite /tmp/ditto-bench.sqlite \
  --pg "${DITTO_POSTGRES_URL}" \
  --mysql "${DITTO_MYSQL_URL}" \
  --audit-ops 5000 \
  --reap-ops 2000
```

也可以加 `--out bench.json` 导出 JSON 报告。

## 指标说明

- `audit_append_ms`：完成 `audit_ops` 次审计写入总耗时
- `audit_append_ops_per_sec`：审计写入吞吐
- `audit_cleanup_ms`：清理旧审计日志耗时
- `audit_cleanup_deleted`：清理删除条数
- `reap_ms`：完成 budget+cost 两类 reap 的总耗时
- `budget_* / cost_*`：回收扫描数、回收数、释放额度

> 注意：该基准用于回归对比（同机器、同参数）更有意义，不建议当作跨环境绝对性能排名。
> CI 会在 `gateway-store-bench` job 里跑 sqlite+postgres+mysql 并上传 `store-bench.json` 作为可比对产物。

## 最近一次基线（2026-03-04）

命令：

```bash
cargo run --features "gateway gateway-store-sqlite" --bin ditto-store-bench -- \
  --sqlite /tmp/ditto-bench.sqlite \
  --audit-ops 3000 \
  --reap-ops 1500
```

结果（当前开发机）：

- store: `sqlite`
- audit append: `5890 ms`（约 `509.33 ops/s`）
- audit cleanup: `35 ms`（删除 `3000` 条）
- reap（budget+cost）: `163 ms`
- budget/cost 回收：`1500 / 1500`，释放额度各 `1500`
