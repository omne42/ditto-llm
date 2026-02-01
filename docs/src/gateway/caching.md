# 缓存：Control-plane / Proxy Cache

Ditto Gateway 有两类缓存，面向不同使用场景：

1) **Control-plane cache**：只作用于 `POST /v1/gateway`（demo/控制面端点）。  
2) **Proxy cache**：作用于 OpenAI-compatible passthrough `ANY /v1/*`（非 streaming 响应）。

实现位置：

- Control-plane：`src/gateway/cache.rs` + `src/gateway/mod.rs`
- Proxy cache：`src/gateway/proxy_cache.rs` + `src/gateway/http/proxy/core.rs` + `src/gateway/redis_store/virtual_keys_and_proxy_cache.rs`

---

## 1) Control-plane cache（/v1/gateway）

### 如何开启？

在某个 `VirtualKeyConfig` 下：

```json
{
  "cache": {
    "enabled": true,
    "ttl_seconds": 60,
    "max_entries": 1024
  }
}
```

缓存 key 由如下信息组成（并包含 prompt hash）：

- key id
- model / input_tokens / max_output_tokens
- prompt 的 hash 与长度

### 什么时候会绕过？

`POST /v1/gateway` 有一个 `passthrough` 布尔字段；并且 key 上有：

- `passthrough.allow`
- `passthrough.bypass_cache`

当请求 `passthrough=true` 且 key 允许 passthrough 且 `bypass_cache=true` 时，会绕过 control-plane cache。

> 这套语义主要用于 demo 端点，生产建议更多使用 `/v1/*` passthrough 或 translation。

---

## 2) Proxy cache（/v1/*）

### 2.1 启用条件

Proxy cache 需要：

- 编译启用 feature `gateway-proxy-cache`
- 运行时添加 `--proxy-cache`（或设置 `--proxy-cache-ttl/--proxy-cache-max-entries`）

常用启动例子：

```bash
cargo run --features "gateway gateway-proxy-cache" --bin ditto-gateway -- ./gateway.json \
  --proxy-cache \
  --proxy-cache-ttl 60 \
  --proxy-cache-max-entries 2048 \
  --proxy-cache-max-body-bytes 1048576 \
  --proxy-cache-max-total-body-bytes 67108864
```

> `--proxy-cache-max-body-bytes` 会跳过缓存“过大的响应”（包含 memory 与 redis L2）；`--proxy-cache-max-total-body-bytes` 用于限制内存总缓存体积，避免内存被打爆。

### 2.2 缓存范围（What gets cached）

Ditto 只会缓存：

- 方法：`GET` / `POST`
- 响应：**非** `text/event-stream`（也就是非 streaming）
- 状态：2xx（成功响应）

因此：

- `POST /v1/chat/completions`（非 streaming）可以被缓存
- `POST /v1/chat/completions`（stream=true）不会被缓存
- `GET /v1/models` 可以被缓存

### 2.3 Cache key 与 scope（重要）

Proxy cache key 由以下因素决定（见 `proxy_cache_key`）：

- HTTP method
- path（包含 `/v1/...`）
- body hash
- scope

scope 的选择规则（见 `proxy_cache_scope`）：

- 如果启用 virtual keys：scope = `vk:<virtual_key_id>`
- 否则：
  - 若请求带 `Authorization`：scope = `auth:<hash>`
  - 否则若请求带 `x-api-key`：scope = `x-api-key:<hash>`
  - 否则：`public`

这能避免不同 key/不同上游 token 之间的缓存串用。

### 2.4 如何绕过 proxy cache

任一条件成立即 bypass：

- 请求头包含 `x-ditto-cache-bypass` 或 `x-ditto-bypass-cache`
- `Cache-Control` 包含 `no-store` 或 `no-cache`

> 注意：Ditto 会在转发 upstream 前移除这些 Ditto 私有 header，避免污染上游。

### 2.5 命中时的响应头

当 proxy cache 命中时，你会看到：

- `x-ditto-cache: hit`
- `x-ditto-cache-key: <cache_key>`
- `x-ditto-cache-source: memory|redis`（若同时启用 redis store）

---

## 3) Redis 共享缓存（多副本）

当你同时启用：

- feature `gateway-store-redis`
- 运行时 `--redis <url>`

proxy cache 会：

- 仍写入本机内存（L1）
- 同时写入 redis（L2，共享）

命中时会返回 `x-ditto-cache-source` 指示来源。

---

## 4) 管理：清理缓存（Admin API）

启用 admin token + proxy cache 后：

- `POST /admin/proxy_cache/purge`
  - `{ "all": true }`：清理全部
  - `{ "cache_key": "ditto-proxy-cache-v1-..." }`：按 key 清理

如果启用 redis store，会同时清理 redis 中的记录，并在响应里返回 `deleted_redis`（删除条数）。

---

## 5) 使用建议（好品味版）

- 不要对 streaming 响应做缓存：语义复杂且容易引入“半截缓存”问题；Ditto 直接选择不缓存。
- 对大响应（例如 files/audio download）谨慎开启缓存：它会占用内存与 redis 带宽。
- 如果你需要“更像 CDN 的缓存”，建议把 `/v1/*` 放到边缘缓存层做细粒度策略，Ditto 负责控制面与路由治理。
