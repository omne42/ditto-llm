# JS：Stream Protocol v1 解析（SSE / NDJSON）

`@ditto-llm/client` 提供两个能力：

1) 解析 Ditto 的 **Stream Protocol v1**（见「SDK → Stream Protocol v1」）。  
2) 一个很薄的 **Admin API client**（可选），便于脚本化管理 virtual keys / 审计导出。

> 目前这些包以 workspace 形式存在于仓库 `packages/` 下（`private: true`），主要用于示例与内部集成；如果你要在仓库外使用，可以先 vendor/拷贝实现，或把它当作未来可发布的稳定接口。

---

## 1) 解析 Stream Protocol v1（核心）

### 1.1 事件类型

解析后每个 event 都是一个对象：

- `{ v: 1, type: "chunk", data: <StreamChunk> }`
- `{ v: 1, type: "error", data: { message: string } }`
- `{ v: 1, type: "done" }`

### 1.2 SSE（`data: <json>\n\n`）

```ts
import { streamV1FromSseResponse } from "@ditto-llm/client";

const res = await fetch("http://127.0.0.1:8080/my/stream-v1-sse", {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: JSON.stringify({ input: "hello" }),
});

for await (const evt of streamV1FromSseResponse(res)) {
  if (evt.type === "chunk") {
    // evt.data 是一个 StreamChunk（shape 取决于你的服务端输出）
    console.log("chunk", evt.data);
  } else if (evt.type === "error") {
    console.error("stream error", evt.data.message);
  } else if (evt.type === "done") {
    break;
  }
}
```

### 1.3 NDJSON（`<json>\n`）

```ts
import { streamV1FromNdjsonResponse } from "@ditto-llm/client";

const res = await fetch("http://127.0.0.1:8080/my/stream-v1-ndjson");
for await (const evt of streamV1FromNdjsonResponse(res)) {
  if (evt.type === "done") break;
}
```

### 1.4 自动分发（推荐）

如果你的 API 端点会根据 `content-type`/参数切换 SSE 与 NDJSON，可以统一用：

```ts
import { streamV1FromResponse } from "@ditto-llm/client";

const format = "sse"; // 或 "ndjson"
const res = await fetch("http://127.0.0.1:8080/my/stream", { method: "POST" });
for await (const evt of streamV1FromResponse(res, format)) {
  if (evt.type === "done") break;
}
```

---

## 2) Admin API client（可选）

`createAdminClient` 是一个薄封装，适合脚本/运维工具调用 `ditto-gateway` 的 `/admin/*` 端点。

```ts
import { createAdminClient } from "@ditto-llm/client";

const admin = createAdminClient({
  baseUrl: "http://127.0.0.1:8080",
  token: process.env.DITTO_ADMIN_TOKEN!,
  // 默认使用 Authorization: Bearer <token>
  // header: "x-admin-token",
});

const keys = await admin.listKeys({ limit: 100 });
console.log(keys);
```

可用方法（以源码为准）：

- `health()`
- `listKeys()` / `upsertKey()` / `deleteKey()`
- `listAudit()` / `exportAudit()`

下一步：

- 「Gateway → Admin API」与「Gateway → 鉴权：Virtual Keys 与 Admin Token」。

