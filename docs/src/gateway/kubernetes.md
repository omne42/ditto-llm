# Kubernetes（多副本模板）

本页提供一套最小 Kubernetes manifests，作为企业部署的起点（多副本 + Redis 共享状态）。

模板目录：`deploy/k8s/`

- `configmap.yaml`：`gateway.json`（支持 `${ENV_VAR}` 插值）
- `secret.example.yaml`：示例 Secret（请自行替换）
- `deployment.yaml`：2 副本 Deployment（含 /health 探针）
- `service.yaml`：ClusterIP Service

---

## 1) 构建与推送镜像

在 `ditto-llm` 仓库根目录：

```bash
docker build -t <registry>/ditto-gateway:<tag> .
docker push <registry>/ditto-gateway:<tag>
```

然后把 `deploy/k8s/deployment.yaml` 里的 `image:` 改成你的镜像地址。

---

## 2) 配置 Redis

多副本要想让 virtual keys / budgets / audit / cache 在实例间一致，建议启用 Redis store。

你需要：

- 集群内可访问的 Redis（Service 名例如 `redis:6379`）
- 在 Secret 里设置 `REDIS_URL`

---

## 3) 应用 manifests

```bash
kubectl apply -f deploy/k8s/configmap.yaml
kubectl apply -f deploy/k8s/secret.example.yaml
kubectl apply -f deploy/k8s/deployment.yaml
kubectl apply -f deploy/k8s/service.yaml
```

端口转发验证：

```bash
kubectl port-forward svc/ditto-gateway 8080:8080
curl -sS http://127.0.0.1:8080/health
```

---

## 4) Ingress / LB（streaming 注意事项）

如果你要在 ingress/LB 后面代理 `text/event-stream`：

- 关闭响应缓冲（buffering）
- 调高 idle timeout（streaming 请求会长连接）
- 确保不会把 streaming 响应强制压缩/聚合

具体配置与 controller 相关，这里不做绑定；生产建议把这类配置固化成你的平台层模板。
