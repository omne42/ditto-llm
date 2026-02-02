const baseUrl = process.env.DITTO_BASE_URL ?? "http://127.0.0.1:8080";
const token = process.env.DITTO_VK_TOKEN;

if (!token) {
  console.error("missing DITTO_VK_TOKEN");
  process.exit(1);
}

const res = await fetch(`${baseUrl.replace(/\\/+$/, "")}/v1/chat/completions`, {
  method: "POST",
  headers: {
    "content-type": "application/json",
    authorization: `Bearer ${token}`,
    "x-request-id": `node-${Date.now()}`,
  },
  body: JSON.stringify({
    model: "gpt-4o-mini",
    stream: true,
    messages: [{ role: "user", content: "Say hello in one sentence." }],
  }),
});

if (!res.ok) {
  console.error("HTTP", res.status, await res.text());
  process.exit(1);
}

const reader = res.body.getReader();
const decoder = new TextDecoder();
let buffer = "";
let dataLines = [];

while (true) {
  const { value, done } = await reader.read();
  if (done) break;

  buffer += decoder.decode(value, { stream: true });
  while (true) {
    const idx = buffer.indexOf("\n");
    if (idx === -1) break;
    const line = buffer.slice(0, idx).replace(/\\r$/, "");
    buffer = buffer.slice(idx + 1);

    if (line === "") {
      if (dataLines.length === 0) continue;
      const payload = dataLines.join("\\n");
      dataLines = [];
      if (payload === "[DONE]") {
        process.stdout.write("\\n");
        process.exit(0);
      }
      const evt = JSON.parse(payload);
      const delta = evt?.choices?.[0]?.delta?.content;
      if (typeof delta === "string") process.stdout.write(delta);
      continue;
    }

    if (line.startsWith("data:")) {
      dataLines.push(line.slice(5).trimStart());
    }
  }
}

process.stdout.write("\\n");
