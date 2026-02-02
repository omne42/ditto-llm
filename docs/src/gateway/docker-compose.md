# Docker Compose（本地模板）

本页提供一个“复制即可跑”的 `docker compose` 模板，用于本地/CI 快速把 `ditto-gateway + redis` 跑起来（对标 LiteLLM docs 的 quickstart 体验）。

模板文件：

- `deploy/docker-compose.yml`
- `deploy/gateway.example.json`
- `deploy/.env.example`

---

## 1) 准备环境变量

在仓库根目录：

```bash
cp deploy/.env.example deploy/.env
```

编辑 `deploy/.env`，填入：

- `OPENAI_API_KEY`
- `DITTO_ADMIN_TOKEN`
- `DITTO_VK_BOOTSTRAP`
- `REDIS_URL`（默认 `redis://redis:6379`）

---

## 2) 启动

```bash
docker compose -f deploy/docker-compose.yml up --build
```

默认监听 `http://127.0.0.1:8080`。

---

## 3) 验证

健康检查：

```bash
curl -sS http://127.0.0.1:8080/health
```

OpenAI-compatible（用 virtual key 调用）：

```bash
curl -sS http://127.0.0.1:8080/v1/models -H "Authorization: Bearer ${DITTO_VK_BOOTSTRAP}" | head
```

Admin API（用 admin token）：

```bash
curl -sS http://127.0.0.1:8080/admin/keys -H "Authorization: Bearer ${DITTO_ADMIN_TOKEN}" | head
```

---

## 4) 生产注意事项（简版）

- 多副本建议直接走「部署：多副本与分布式」+ 「Kubernetes 模板」，并启用 redis store 保证一致性。
- 如果你在 ingress/LB 后面跑 streaming（SSE），请确保 proxy 不会对响应做缓冲（buffering），并把 idle timeout 调高。
