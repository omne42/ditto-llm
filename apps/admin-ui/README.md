# ditto-admin-ui

Optional React admin UI asset for `ditto-gateway` (not part of the default core build/CI path):

- List / upsert / delete virtual keys
- View audit logs + export JSONL/CSV
- Works with global admin tokens and tenant-scoped admin tokens

## Run

From repo root:

```bash
pnpm install
pnpm run dev:admin-ui
```

Then open `http://127.0.0.1:5173`.
