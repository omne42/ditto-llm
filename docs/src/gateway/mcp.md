# MCP Gateway（/mcp + tools）

Ditto Gateway 提供一个 **LiteLLM-like** 的 MCP HTTP JSON-RPC proxy，并把 MCP tools 融合进 OpenAI-compatible 的 `POST /v1/chat/completions`。

你可以把它理解为两条能力：

1) **MCP Proxy**：`/mcp*` 端点 → 转发 `tools/list` / `tools/call` 到你注册的 MCP servers  
2) **Chat Completions 集成**：在 `tools` 里写 `{"type":"mcp", ...}`，Ditto 会把 MCP tools 转成 OpenAI `function` tools，并可选自动执行 tool calls

---

## 1) 配置：mcp_servers registry

在 `gateway.json`（或启用 `gateway-config-yaml` 后的 `gateway.yaml`）里注册 MCP servers：

```json
{
  "mcp_servers": [
    {
      "server_id": "local",
      "url": "http://127.0.0.1:3000/mcp",
      "headers": { "authorization": "Bearer ${MCP_TOKEN}" },
      "query_params": {},
      "timeout_seconds": 30
    }
  ]
}
```

字段说明：

- `server_id`：server 的逻辑 id（用于选择与工具名加前缀）
- `url`：MCP server 的 HTTP endpoint（只支持 `http://` / `https://`）
- `headers` / `query_params`：转发时注入（可用于鉴权）
- `timeout_seconds`：覆盖默认超时（默认 300s）

兼容性补充：

- `url` 也接受别名字段 `http_url`（迁移/对齐时更顺手）
- 上述字段支持 `${ENV}` / `os.environ/ENV` / `secret://...`（env/secret 缺失会启动失败，避免 silent misconfig）

---

## 2) 作为 MCP Proxy 使用（/mcp）

### 2.1 tools/list

默认会聚合 **全部** 已配置的 MCP servers：

```bash
curl -sS http://127.0.0.1:8080/mcp/tools/list \
  -H 'content-type: application/json' \
  -d '{"servers":["local"]}'
```

也可以直接走 MCP JSON-RPC：

```bash
curl -sS http://127.0.0.1:8080/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

### 2.2 tools/call

```bash
curl -sS http://127.0.0.1:8080/mcp/tools/call \
  -H 'content-type: application/json' \
  -d '{"server_id":"local","name":"hello","arguments":{"who":"world"}}'
```

---

## 3) 选择 MCP servers（多 server 支持）

默认行为：未显式指定时，会使用 **全部** `mcp_servers`。

显式指定 server 的方式（任选其一）：

- HTTP header：`x-mcp-servers: local,github`
- URL path selector：
  - `/mcp/local,github`
  - `/local,github/mcp`

当一次选择多个 servers 时，为了避免工具名冲突，Ditto 会给工具名加前缀：

- `hello` → `local-hello`

随后 `tools/call` 也需要用同样的前缀形式（`<server_id>-<tool_name>`）。

---

## 4) 在 /v1/chat/completions 中使用 MCP tools

Ditto 支持 LiteLLM 风格的请求写法：在 `tools` 数组中放入 `type: "mcp"` 的条目。

最小示例：

```json
{
  "model": "gpt-4o-mini",
  "messages": [{ "role": "user", "content": "Call hello tool." }],
  "tools": [
    {
      "type": "mcp",
      "server_url": "litellm_proxy/mcp/local",
      "allowed_tools": ["hello"]
    }
  ]
}
```

行为：

- Ditto 会对选中的 MCP server(s) 执行 `tools/list`
- 将 MCP tool schema 转换为 OpenAI `function` tools
- 把转换后的 `tools` 发送给 upstream（passthrough 或 translation backend）

### 4.1 自动执行（require_approval: "never"）

当任意 MCP tool config 中包含：

```json
{ "type": "mcp", "require_approval": "never" }
```

Ditto 会做一次最小 tool loop（对齐 LiteLLM 的行为）：

1) 先用 `stream=false` 调一次 `/v1/chat/completions` 获取 `tool_calls`
2) 逐个调用 MCP `tools/call`
3) 把结果作为 `role:"tool"` message 追加回 `messages`
4) 再发起一次最终请求（最终请求的 `stream` 会保持和原请求一致）

> 当前只拦截 `POST /v1/chat/completions` 的 MCP tools；`/v1/responses` 的 MCP tools 暂未接入。

---

## 5) 鉴权（Virtual Keys）

如果你的 `gateway.json` 里配置了 `virtual_keys`（即非空），那么：

- `/mcp*` 端点同样需要 virtual key
- 兼容以下 header（优先级从高到低）：
  1) `x-litellm-api-key`（支持 `Bearer ...` 前缀）
  2) `Authorization: Bearer ...`
  3) `x-ditto-virtual-key`
  4) `x-api-key`

---

## 6) 已实现范围与差异（务实口径）

Ditto 的 MCP gateway 当前聚焦在“让 MCP tools 能跑起来”：

- ✅ HTTP JSON-RPC：`initialize` / `tools/list` / `tools/call`
- ✅ LiteLLM-like 路由：`/mcp`、`/mcp/<servers>`、`/<servers>/mcp`、`x-mcp-servers`
- ✅ tools → OpenAI function tools 转换（`/v1/chat/completions`）
- ✅ `allowed_tools`（请求级过滤；支持带/不带 `<server_id>-` 前缀）

未覆盖项（如果你需要，可以作为后续切片推进）：

- per-key/team/org 的 MCP 权限管理、`allowed_params` 等更细粒度策略（LiteLLM 有更完整的控制面）
- streaming cache / 更复杂的审批流

