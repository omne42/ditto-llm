import os
import sys
import json
import urllib.request


def main() -> int:
    base_url = os.environ.get("DITTO_BASE_URL", "http://127.0.0.1:8080").rstrip("/")
    token = os.environ.get("DITTO_VK_TOKEN")
    if not token:
        print("missing DITTO_VK_TOKEN", file=sys.stderr)
        return 1

    payload = {
        "model": "gpt-4o-mini",
        "stream": False,
        "messages": [{"role": "user", "content": "Say hello in one sentence."}],
    }

    req = urllib.request.Request(
        f"{base_url}/v1/chat/completions",
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "content-type": "application/json",
            "authorization": f"Bearer {token}",
            "x-request-id": f"py-{int(__import__('time').time() * 1000)}",
        },
        method="POST",
    )

    with urllib.request.urlopen(req) as resp:
        body = resp.read().decode("utf-8")
        print(body)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
