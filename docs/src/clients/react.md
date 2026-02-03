# React：useStreamV1

`@ditto-llm/react` 提供一个轻量 hook：`useStreamV1()`，用于把 Stream Protocol v1 的事件流聚合成适合 UI 使用的状态。

它适合两类场景：

- 你在浏览器里直接请求一个返回 **stream protocol v1（SSE/NDJSON）** 的端点。
- 你在 React App 里通过自建 `/api/*` 代理到 Ditto（或你的服务端 Ditto SDK）。

> 这个 hook 不依赖特定框架，不等同于 AI SDK UI 的完整 `useChat` 生态；目标是提供“最小、可控、易排障”的 streaming UI 基建。

---

## 1) 最小用法

```tsx
import { useStreamV1 } from "@ditto-llm/react";

export function Demo() {
  const { state, start, abort } = useStreamV1();

  return (
    <div>
      <button
        onClick={() =>
          start(
            (signal) =>
              fetch("/api/stream", {
                method: "POST",
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ input: "hello" }),
                signal,
              }),
            "sse",
          )
        }
        disabled={state.isLoading}
      >
        Start
      </button>
      <button onClick={abort} disabled={!state.isLoading}>
        Abort
      </button>

      {state.error && <pre>Error: {state.error}</pre>}
      <pre>{state.text}</pre>
    </div>
  );
}
```

`start(createResponse, format)` 参数说明：

- `createResponse(signal)`：你提供一个能创建 `fetch()` Response 的函数（hook 会传入 AbortSignal）。
- `format`：`"sse"` 或 `"ndjson"`，与服务端输出一致。

---

## 2) State 字段

`state` 会持续更新：

- `text`：当收到 `text_delta` chunk 时自动拼接（best-effort）
- `chunks`：原始 `StreamChunk` 列表（用于 debug / 自定义 UI）
- `warnings`：聚合后的 warnings（当服务端输出 warnings chunk）
- `done` / `isLoading` / `error`
- `responseId` / `finishReason` / `usage`：best-effort 提取（具体 shape 取决于服务端输出）

下一步：

- 「SDK → Stream Protocol v1」了解服务端如何输出协议。
- 如果你希望兼容 Vercel AI SDK UI：看同页的 UI Message Stream v1 适配说明。

