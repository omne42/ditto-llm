# ditto-gateway Helm chart

This chart deploys `ditto-gateway` (Ditto-LLM gateway HTTP service) with:

- A `ConfigMap` containing `gateway.json`
- An optional `Secret` for env vars (or reference an existing one)
- A `Deployment` + `Service`
- An optional `ServiceMonitor` (Prometheus Operator)

## Quickstart

```bash
helm install ditto-gateway ./deploy/helm/ditto-gateway \
  --set image.repository=<registry>/ditto-gateway \
  --set image.tag=<tag> \
  --set secret.create=true \
  --set-string secret.stringData.OPENAI_API_KEY=sk-... \
  --set-string secret.stringData.DITTO_VK_BOOTSTRAP=ditto-vk-... \
  --set-string secret.stringData.DITTO_ADMIN_TOKEN=ditto-admin-... \
  --set-string secret.stringData.REDIS_URL=redis://redis:6379
```

Then:

```bash
kubectl port-forward svc/ditto-gateway 8080:8080
curl -sS http://127.0.0.1:8080/health
```
