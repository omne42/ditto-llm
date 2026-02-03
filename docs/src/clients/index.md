# 客户端（JS/React，AI SDK UI-like）

本章面向 **前端/Node** 使用场景：当你的服务端把 Ditto 的 streaming 暴露为 **Stream Protocol v1**（见「SDK → Stream Protocol v1」）时，你需要一个能消费该协议的客户端实现。

Ditto 的定位是提供“最小可用”的 DX：

- 不试图复刻完整的 AI SDK UI 生态（RSC/框架适配/高级 hooks）。
- 提供可靠的协议解析（SSE/NDJSON）与一个轻量的 React hook，便于快速接入 UI。

如果你希望直接对接 Vercel AI SDK UI（例如 `@ai-sdk/react` 的 `useChat`），请优先看「SDK → Stream Protocol v1」里关于 **UI Message Stream v1** 的适配器说明（它与 Ditto 的 stream protocol v1 不同）。

本章包括：

- 「JS：Stream Protocol v1 解析」：`@ditto-llm/client`
- 「React：useStreamV1」：`@ditto-llm/react`

