export type StreamEventV1 =
  | { v: 1; type: "chunk"; data: unknown }
  | { v: 1; type: "error"; data: { message: string } }
  | { v: 1; type: "done" };

export type StreamChunk = Record<string, unknown> & { type?: string };

function normalizeBaseUrl(baseUrl: string): string {
  return baseUrl.replace(/\/+$/, "");
}

async function* readLines(stream: ReadableStream<Uint8Array>): AsyncIterable<string> {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });
    while (true) {
      const idx = buffer.indexOf("\n");
      if (idx === -1) break;
      const line = buffer.slice(0, idx);
      buffer = buffer.slice(idx + 1);
      yield line.replace(/\r$/, "");
    }
  }

  buffer += decoder.decode();
  if (buffer.length > 0) {
    for (const line of buffer.split("\n")) {
      const trimmed = line.replace(/\r$/, "");
      if (trimmed.length > 0) yield trimmed;
    }
  }
}

export async function* streamV1FromNdjsonResponse(res: Response): AsyncIterable<StreamEventV1> {
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${text || res.statusText}`);
  }
  if (!res.body) {
    throw new Error("missing response body");
  }

  for await (const line of readLines(res.body)) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const event = JSON.parse(trimmed) as StreamEventV1;
    yield event;
    if (event.type === "done") return;
  }
}

export async function* streamV1FromSseResponse(res: Response): AsyncIterable<StreamEventV1> {
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${text || res.statusText}`);
  }
  if (!res.body) {
    throw new Error("missing response body");
  }

  let dataLines: string[] = [];
  for await (const line of readLines(res.body)) {
    if (line === "") {
      if (dataLines.length > 0) {
        const payload = dataLines.join("\n");
        dataLines = [];
        if (payload === "[DONE]") {
          yield { v: 1, type: "done" };
          return;
        }
        const event = JSON.parse(payload) as StreamEventV1;
        yield event;
        if (event.type === "done") return;
      }
      continue;
    }

    if (line.startsWith("data:")) {
      dataLines.push(line.slice(5).trimStart());
    }
  }
}

export type StreamV1Format = "sse" | "ndjson";

export async function* streamV1FromResponse(
  res: Response,
  format: StreamV1Format,
): AsyncIterable<StreamEventV1> {
  if (format === "ndjson") {
    yield* streamV1FromNdjsonResponse(res);
    return;
  }
  yield* streamV1FromSseResponse(res);
}

export interface AdminClientOptions {
  baseUrl: string;
  token: string;
  header?: "authorization" | "x-admin-token";
}

export interface ListKeysOptions {
  includeTokens?: boolean;
  tenantId?: string;
  projectId?: string;
  userId?: string;
  enabled?: boolean;
  idPrefix?: string;
  limit?: number;
  offset?: number;
}

export interface ListAuditOptions {
  limit?: number;
  sinceTsMs?: number;
  beforeTsMs?: number;
  tenantId?: string;
}

export interface ExportAuditOptions extends ListAuditOptions {
  format?: "jsonl" | "csv";
}

export function createAdminClient(opts: AdminClientOptions) {
  const baseUrl = normalizeBaseUrl(opts.baseUrl);
  const header = opts.header ?? "authorization";

  function withAuth(init: RequestInit = {}): RequestInit {
    const headers = new Headers(init.headers);
    if (header === "authorization") {
      headers.set("authorization", `Bearer ${opts.token}`);
    } else {
      headers.set("x-admin-token", opts.token);
    }
    return { ...init, headers };
  }

  async function request(path: string, init?: RequestInit): Promise<Response> {
    return fetch(`${baseUrl}${path}`, withAuth(init));
  }

  function qs(params: Record<string, string | number | boolean | undefined | null>): string {
    const out = new URLSearchParams();
    for (const [k, v] of Object.entries(params)) {
      if (v === undefined || v === null) continue;
      out.set(k, String(v));
    }
    const s = out.toString();
    return s ? `?${s}` : "";
  }

  return {
    async health(): Promise<unknown> {
      const res = await fetch(`${baseUrl}/health`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      return res.json();
    },

    async listKeys(options: ListKeysOptions = {}): Promise<unknown[]> {
      const res = await request(
        `/admin/keys${qs({
          include_tokens: options.includeTokens,
          tenant_id: options.tenantId,
          project_id: options.projectId,
          user_id: options.userId,
          enabled: options.enabled,
          id_prefix: options.idPrefix,
          limit: options.limit,
          offset: options.offset,
        })}`,
      );
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      return res.json();
    },

    async upsertKey(key: unknown): Promise<unknown> {
      const res = await request("/admin/keys", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(key),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      return res.json();
    },

    async deleteKey(id: string): Promise<void> {
      const res = await request(`/admin/keys/${encodeURIComponent(id)}`, { method: "DELETE" });
      if (!res.ok && res.status !== 404) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
    },

    async listAudit(options: ListAuditOptions = {}): Promise<unknown[]> {
      const res = await request(
        `/admin/audit${qs({
          limit: options.limit,
          since_ts_ms: options.sinceTsMs,
          before_ts_ms: options.beforeTsMs,
          tenant_id: options.tenantId,
        })}`,
      );
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${await res.text()}`);
      return res.json();
    },

    async exportAudit(options: ExportAuditOptions = {}): Promise<Response> {
      const format = options.format ?? "jsonl";
      return request(
        `/admin/audit/export${qs({
          format,
          limit: options.limit,
          since_ts_ms: options.sinceTsMs,
          before_ts_ms: options.beforeTsMs,
          tenant_id: options.tenantId,
        })}`,
      );
    },
  };
}
