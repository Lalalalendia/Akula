#!/usr/bin/env python3
"""Test-only HTTPS reverse proxy to a ONE-OBJECT anonymous MinIO S3 origin.

Converts the CI-local HTTP MinIO object endpoint into an HTTPS origin a
browser can fetch directly. Not a generic S3 proxy, does not accept writes.
"""
import argparse
import asyncio
import json
import ssl
from pathlib import Path
from aiohttp import web, ClientSession, ClientTimeout

S3_ENDPOINT = "http://127.0.0.1:9000"
ALLOW_ORIGIN = "http://127.0.0.1:8765"
BUFFER = 64 * 1024
STATE_KEY = web.AppKey("bridge", dict)


def make_app(bucket, key):
    app = web.Application(client_max_size=1024)
    state = {"bytes": 0, "gets": 0, "ranges": 0, "bucket": bucket, "key": key}
    app[STATE_KEY] = state
    cors = {
        "Access-Control-Allow-Origin": ALLOW_ORIGIN,
        "Access-Control-Allow-Methods": "GET, HEAD, OPTIONS",
        "Access-Control-Allow-Headers": "Range, If-Match, If-Range",
        "Access-Control-Expose-Headers": "Content-Range, Content-Length, Accept-Ranges, ETag, Content-Type",
        "Vary": "Origin", "Cache-Control": "no-store",
    }

    async def stats(_):
        return web.json_response({"origin_bytes": state["bytes"],
                                  "origin_gets": state["gets"],
                                  "range_gets": state["ranges"],
                                  "origin": "minio-public-s3-via-https-test-gateway"},
                                 headers={"Cache-Control": "no-store"})

    async def proxy(request):
        if request.match_info["bucket"] != bucket or request.match_info["key"] != key:
            raise web.HTTPNotFound()
        if request.method == "OPTIONS":
            return web.Response(status=204, headers=cors)
        if request.method not in {"GET", "HEAD"}:
            raise web.HTTPMethodNotAllowed(request.method, ["GET", "HEAD", "OPTIONS"])
        headers = {}
        for h in ("Range", "If-Match", "If-Range"):
            if h in request.headers:
                headers[h] = request.headers[h]
        timeout = ClientTimeout(total=120, sock_read=30)
        async with ClientSession(timeout=timeout, trust_env=False) as s:
            async with s.request(request.method, f"{S3_ENDPOINT}/{bucket}/{key}",
                                 headers=headers, allow_redirects=False) as origin:
                out = dict(cors)
                for name in ("Content-Type", "Content-Length", "Content-Range",
                             "Accept-Ranges", "ETag", "Last-Modified"):
                    if name in origin.headers:
                        out[name] = origin.headers[name]
                if request.method == "HEAD":
                    return web.Response(status=origin.status, headers=out)
                state["gets"] += 1
                if "Range" in headers:
                    state["ranges"] += 1
                res = web.StreamResponse(status=origin.status, headers=out)
                await res.prepare(request)
                async for block in origin.content.iter_chunked(BUFFER):
                    if not block:
                        continue
                    await res.write(block)
                    state["bytes"] += len(block)
                await res.write_eof()
                return res

    app.router.add_get("/metrics", stats)
    app.router.add_route("GET", "/{bucket}/{key:.+}", proxy)
    app.router.add_route("HEAD", "/{bucket}/{key:.+}", proxy)
    app.router.add_route("OPTIONS", "/{bucket}/{key:.+}", proxy)
    return app


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cert", type=Path, required=True)
    ap.add_argument("--key", type=Path, required=True)
    ap.add_argument("--config", type=Path, default=Path("test-results/minio-browser-config.json"))
    args = ap.parse_args()
    config = json.loads(args.config.read_text(encoding="utf-8"))
    assert config["bucket"].startswith("akula-public-webrtc-")
    assert config["key"] == "immutable/8m.dat"
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(args.cert, args.key)
    web.run_app(make_app(config["bucket"], config["key"]),
                host="127.0.0.1", port=9443, ssl_context=ctx, print=None)


if __name__ == "__main__":
    main()
