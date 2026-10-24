#!/usr/bin/env python
"""Safe field extractor for e2e-api.mjs JSON output: {"status": int, "body": str|"<truncated>"}.
Usage: echo <json> | python jget.py <status|url|exists|size|meta-updated> [needle]
No eval: each field is an explicit code path."""
import json, sys

def main():
    d = json.load(sys.stdin)
    field = sys.argv[1] if len(sys.argv) > 1 else "status"
    body = {}
    if isinstance(d.get("body"), str):
        try:
            body = json.loads(d["body"])
        except Exception:
            body = {}
    elif isinstance(d.get("body"), dict):
        body = d["body"]
    if field == "status":
        print(d.get("status", "?"))
    elif field == "url":
        print(body.get("url", ""))
    elif field == "exists":
        print(body.get("exists", "?"))
    elif field == "size":
        print(body.get("size", "?"))
    elif field == "meta-updated":
        print((body.get("metadata") or {}).get("updated-at", ""))
    elif field == "list-count":
        needle = sys.argv[2] if len(sys.argv) > 2 else ""
        objs = body.get("objects", body.get("contents", []))
        n = 0
        for o in objs:
            k = o.get("key") or o.get("Key") or ""
            if needle in k:
                n += 1
        print(n)
    else:
        print("?")

main()
