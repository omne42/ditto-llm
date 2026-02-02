import { useEffect, useMemo, useState } from "react";

import { createAdminClient } from "@ditto-llm/client";

type AdminHeaderMode = "authorization" | "x-admin-token";

function downloadBlob(filename: string, blob: Blob) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

function storageGet(key: string, fallback: string) {
  const v = localStorage.getItem(key);
  return v ?? fallback;
}

export function App() {
  const [baseUrl, setBaseUrl] = useState(() => storageGet("ditto.baseUrl", "http://127.0.0.1:8080"));
  const [token, setToken] = useState(() => storageGet("ditto.adminToken", ""));
  const [headerMode, setHeaderMode] = useState<AdminHeaderMode>(() =>
    (storageGet("ditto.headerMode", "authorization") as AdminHeaderMode) ?? "authorization",
  );

  const [tenantId, setTenantId] = useState(() => storageGet("ditto.tenantId", ""));
  const [includeTokens, setIncludeTokens] = useState(false);

  const [keys, setKeys] = useState<unknown[] | null>(null);
  const [audit, setAudit] = useState<unknown[] | null>(null);
  const [auditLimit, setAuditLimit] = useState(100);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [keyJson, setKeyJson] = useState(() =>
    JSON.stringify(
      {
        id: "vk-dev",
        token: "ditto-vk-...",
        enabled: true,
        tenant_id: tenantId || null,
        project_id: null,
        user_id: null,
        tenant_budget: null,
        project_budget: null,
        user_budget: null,
        tenant_limits: null,
        project_limits: null,
        user_limits: null,
        limits: { rpm: 60, tpm: 20000 },
        budget: { total_tokens: 1000000, total_usd_micros: null },
        cache: {
          enabled: false,
          ttl_seconds: null,
          max_entries: 1024,
          max_body_bytes: 1048576,
          max_total_body_bytes: 67108864,
        },
        guardrails: { block_pii: true, validate_schema: true },
        passthrough: { allow: true, bypass_cache: true },
        route: null,
      },
      null,
      2,
    ),
  );

  useEffect(() => {
    localStorage.setItem("ditto.baseUrl", baseUrl);
  }, [baseUrl]);
  useEffect(() => {
    localStorage.setItem("ditto.adminToken", token);
  }, [token]);
  useEffect(() => {
    localStorage.setItem("ditto.headerMode", headerMode);
  }, [headerMode]);
  useEffect(() => {
    localStorage.setItem("ditto.tenantId", tenantId);
  }, [tenantId]);

  const client = useMemo(() => {
    if (!token.trim()) return null;
    return createAdminClient({ baseUrl, token, header: headerMode });
  }, [baseUrl, token, headerMode]);

  async function run<T>(label: string, fn: () => Promise<T>): Promise<T | null> {
    setBusy(label);
    setError(null);
    try {
      return await fn();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return null;
    } finally {
      setBusy(null);
    }
  }

  async function loadKeys() {
    if (!client) {
      setError("missing admin token");
      return;
    }
    const data = await run("loadKeys", () =>
      client.listKeys({
        tenantId: tenantId.trim() ? tenantId.trim() : undefined,
        includeTokens,
        limit: 1000,
      }),
    );
    if (data) setKeys(data);
  }

  async function deleteKey(id: string) {
    if (!client) return;
    const ok = window.confirm(`Delete key ${id}?`);
    if (!ok) return;
    await run("deleteKey", async () => {
      await client.deleteKey(id);
      await loadKeys();
      return null;
    });
  }

  async function upsertKey() {
    if (!client) return;
    await run("upsertKey", async () => {
      const parsed = JSON.parse(keyJson);
      const result = await client.upsertKey(parsed);
      await loadKeys();
      return result;
    });
  }

  async function loadAudit() {
    if (!client) return;
    const data = await run("loadAudit", () =>
      client.listAudit({
        limit: auditLimit,
        tenantId: tenantId.trim() ? tenantId.trim() : undefined,
      }),
    );
    if (data) setAudit(data);
  }

  async function exportAudit(format: "jsonl" | "csv") {
    if (!client) return;
    const res = await run("exportAudit", () =>
      client.exportAudit({
        format,
        limit: auditLimit,
        tenantId: tenantId.trim() ? tenantId.trim() : undefined,
      }),
    );
    if (!res) return;
    if (!res.ok) {
      setError(`HTTP ${res.status}: ${await res.text()}`);
      return;
    }
    const blob = await res.blob();
    downloadBlob(`ditto-audit.${format}`, blob);
  }

  return (
    <div className="container">
      <div className="header">
        <div>
          <div className="title">Ditto Gateway Admin</div>
          <div className="subtitle">Keys / audit / export (works with global or tenant-scoped tokens)</div>
        </div>
        <div className="hint">
          Base URL: <span className="pill">{baseUrl}</span>
        </div>
      </div>

      <div className="grid two">
        <div className="panel">
          <h2>Connection</h2>
          <div className="row">
            <label>Base URL</label>
            <input
              style={{ flex: 1, minWidth: 260 }}
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder="http://127.0.0.1:8080"
            />
          </div>
          <div className="row">
            <label>Admin Token</label>
            <input
              style={{ flex: 1, minWidth: 260 }}
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="ditto-admin-..."
            />
          </div>
          <div className="row">
            <label>Header</label>
            <select value={headerMode} onChange={(e) => setHeaderMode(e.target.value as AdminHeaderMode)}>
              <option value="authorization">Authorization: Bearer ...</option>
              <option value="x-admin-token">x-admin-token: ...</option>
            </select>
            <label>tenant_id (optional)</label>
            <input value={tenantId} onChange={(e) => setTenantId(e.target.value)} placeholder="tenant-a" />
            <label>
              <input type="checkbox" checked={includeTokens} onChange={(e) => setIncludeTokens(e.target.checked)} /> include_tokens
            </label>
            <button className="secondary" disabled={!token.trim() || busy !== null} onClick={() => run("health", () => client!.health()).then(() => {})}>
              Health
            </button>
          </div>
          <div className="hint">
            Tip: tenant-scoped tokens can only see/manage their tenant’s keys/budgets/costs/audit; cross-tenant access is rejected.
          </div>
          {error && <div className="error">{error}</div>}
        </div>

        <div className="panel">
          <h2>Audit</h2>
          <div className="row">
            <label>limit</label>
            <input
              type="number"
              min={1}
              max={1000}
              value={auditLimit}
              onChange={(e) => setAuditLimit(Number(e.target.value))}
            />
            <button disabled={!client || busy !== null} onClick={loadAudit}>
              Load
            </button>
            <button className="secondary" disabled={!client || busy !== null} onClick={() => exportAudit("jsonl")}>
              Export JSONL
            </button>
            <button className="secondary" disabled={!client || busy !== null} onClick={() => exportAudit("csv")}>
              Export CSV
            </button>
          </div>
          <div className="hint">
            Export includes a hash-chain (`prev_hash`/`hash`) in JSONL for tamper evidence. Use the Rust verifier: <span className="pill">ditto-audit-verify</span>.
          </div>
          {audit && (
            <div style={{ maxHeight: 280, overflow: "auto", marginTop: 12 }}>
              <pre style={{ margin: 0, fontSize: 12, color: "var(--muted)" }}>
                {JSON.stringify(audit, null, 2)}
              </pre>
            </div>
          )}
        </div>
      </div>

      <div className="grid two" style={{ marginTop: 16 }}>
        <div className="panel">
          <h2>Keys</h2>
          <div className="row">
            <button disabled={!client || busy !== null} onClick={loadKeys}>
              Load keys
            </button>
          </div>
          {keys && (
            <div style={{ overflow: "auto", marginTop: 12 }}>
              <table className="table">
                <thead>
                  <tr>
                    <th>id</th>
                    <th>tenant_id</th>
                    <th>enabled</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {keys.map((k: any) => (
                    <tr key={String(k.id)}>
                      <td style={{ fontFamily: "ui-monospace" }}>{String(k.id)}</td>
                      <td>{k.tenant_id ? String(k.tenant_id) : <span className="pill">none</span>}</td>
                      <td>{k.enabled ? "true" : "false"}</td>
                      <td>
                        <button className="danger" disabled={busy !== null} onClick={() => deleteKey(String(k.id))}>
                          Delete
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>

        <div className="panel">
          <h2>Upsert Key</h2>
          <div className="row">
            <button disabled={!client || busy !== null} onClick={upsertKey}>
              Upsert
            </button>
            <span className="hint">
              Send a full <span className="pill">VirtualKeyConfig</span> JSON to <span className="pill">POST /admin/keys</span>.
            </span>
          </div>
          <textarea value={keyJson} onChange={(e) => setKeyJson(e.target.value)} />
        </div>
      </div>

      <div className="hint" style={{ marginTop: 16 }}>
        Busy: <span className="pill">{busy ?? "idle"}</span>
      </div>
    </div>
  );
}
