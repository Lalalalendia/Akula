#!/usr/bin/env python3
"""Development-only signaling tracker, static page and local S3-like HTTP Range origin."""
import argparse
import asyncio
import json
import secrets
import re
from collections import defaultdict
from pathlib import Path
from aiohttp import web, WSMsgType
from create_manifest import make_manifest

HERE = Path(__file__).resolve().parent
CHUNK_LIMIT = 4096
HEX = re.compile(r"^[0-9a-f]{64}$")
STATE_KEY = web.AppKey("p2p_state", dict)


def ensure_demo(path: Path, length: int = 8 * 1024 * 1024):
    if path.exists() and path.stat().st_size == length:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    seed = bytes(range(256)) * 256  # deterministic 64KiB, clearly synthetic bytes
    with path.open("wb") as file:
        remaining = length
        while remaining:
            fragment = seed[:min(remaining, len(seed))]
            file.write(fragment)
            remaining -= len(fragment)


def create_app(manifest_path: Path, demo_file: Path | None = None):
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assert HEX.fullmatch(manifest["content_id"])
    assert isinstance(manifest["size"], int) and manifest["size"] >= 0
    assert len(manifest["chunks"]) <= CHUNK_LIMIT
    assert sum(c["size"] for c in manifest["chunks"]) == manifest["size"]
    assert all(HEX.fullmatch(c["sha256"]) for c in manifest["chunks"])
    assert manifest["origin_url"].startswith(("https://", "/origin/"))

    app = web.Application(client_max_size=65536)
    state = {"manifest": manifest, "peers": {}, "origin_bytes": 0, "origin_requests": 0, "signal_messages": 0}
    app[STATE_KEY] = state

    async def index(_):
        return web.FileResponse(HERE / "web/index.html")

    async def client_js(_):
        return web.FileResponse(HERE / "web/client.js", headers={"Cache-Control": "no-store"})

    async def sha256_js(_):
        return web.FileResponse(HERE / "web/sha256_stream.js", headers={"Cache-Control": "no-store"})

    async def bounded_output_js(_):
        return web.FileResponse(HERE / "web/bounded_output.js", headers={"Cache-Control": "no-store"})

    async def range_reader_js(_):
        return web.FileResponse(HERE / "web/range_reader.js", headers={"Cache-Control": "no-store"})

    async def resumable_js(_):
        return web.FileResponse(HERE / "web/resumable_output.js", headers={"Cache-Control": "no-store"})

    async def css(_):
        return web.FileResponse(HERE / "web/style.css")

    async def get_manifest(_):
        return web.json_response(state["manifest"], headers={"Cache-Control": "no-store"})

    async def stats(_):
        return web.json_response({
            "origin_bytes": state["origin_bytes"],
            "origin_requests": state["origin_requests"],
            "signaling_messages": state["signal_messages"],
            "connected_peers": len(state["peers"]),
        })

    async def origin(request):
        if demo_file is None or state["manifest"]["origin_url"] != "/origin/demo.bin":
            raise web.HTTPNotFound()
        size = demo_file.stat().st_size
        if request.method not in ("GET", "HEAD"):
            raise web.HTTPMethodNotAllowed(request.method, ["GET", "HEAD"])
        range_header = request.headers.get("Range")
        start, end = 0, size - 1
        status = 200
        if range_header is not None:
            match = re.fullmatch(r"bytes=(\d+)-(\d*)", range_header)
            if not match or size == 0:
                raise web.HTTPRequestRangeNotSatisfiable(headers={"Content-Range": f"bytes */{size}"})
            start = int(match.group(1))
            end = int(match.group(2)) if match.group(2) else size - 1
            if start >= size or end < start or end >= size:
                raise web.HTTPRequestRangeNotSatisfiable(headers={"Content-Range": f"bytes */{size}"})
            status = 206
        count = max(0, end - start + 1)
        headers = {"Accept-Ranges": "bytes", "Content-Length": str(count), "Content-Type": "application/octet-stream", "Cache-Control": "no-store"}
        if status == 206:
            headers["Content-Range"] = f"bytes {start}-{end}/{size}"
        if request.method == "HEAD":
            return web.Response(status=status, headers=headers)
        state["origin_requests"] += 1
        response = web.StreamResponse(status=status, headers=headers)
        await response.prepare(request)
        with demo_file.open("rb") as file:
            file.seek(start)
            remaining = count
            while remaining:
                data = file.read(min(65536, remaining))
                if not data:
                    break
                try:
                    await response.write(data)
                except (ConnectionResetError, BrokenPipeError):
                    break
                state["origin_bytes"] += len(data)
                remaining -= len(data)
        try:
            await response.write_eof()
        except ConnectionResetError:
            pass
        return response

    async def emit(peer, payload):
        if not peer["ws"].closed:
            await peer["ws"].send_json(payload)

    async def websocket(request):
        ws = web.WebSocketResponse(heartbeat=20, max_msg_size=65536)
        await ws.prepare(request)
        peer_id = secrets.token_hex(8)
        peer = {"id": peer_id, "ws": ws, "content_id": None, "have": set(), "share": False}
        state["peers"][peer_id] = peer
        await emit(peer, {"type": "welcome", "id": peer_id})
        try:
            async for message in ws:
                if message.type != WSMsgType.TEXT:
                    if message.type == WSMsgType.ERROR:
                        break
                    continue
                try:
                    data = json.loads(message.data)
                    if not isinstance(data, dict):
                        continue
                    kind = data.get("type")
                    if kind == "join":
                        cid = data.get("content_id")
                        if cid != state["manifest"]["content_id"]:
                            await emit(peer, {"type": "error", "message": "Unknown ContentID"})
                            continue
                        peer["content_id"] = cid
                        peer["have"] = set()
                        peer["share"] = False
                        others = [{"id": p["id"], "have": sorted(p["have"])} for p in state["peers"].values()
                                  if p["id"] != peer_id and p["content_id"] == cid and p["share"] and p["have"]]
                        await emit(peer, {"type": "joined", "peers": others})
                    elif kind == "announce" and peer["content_id"]:
                        arr = data.get("have")
                        share = data.get("share") is True
                        if not isinstance(arr, list) or len(arr) > CHUNK_LIMIT or any(type(n) is not int or n < 0 or n >= len(manifest["chunks"]) for n in arr):
                            continue
                        peer["have"] = set(arr) if share else set()
                        peer["share"] = share
                        event = {"type": "availability", "id": peer_id, "have": sorted(peer["have"])}
                        for other in list(state["peers"].values()):
                            if other["id"] != peer_id and other["content_id"] == peer["content_id"]:
                                await emit(other, event)
                    elif kind == "signal" and peer["content_id"]:
                        target = state["peers"].get(data.get("to"))
                        signal = data.get("signal")
                        if not (target and target["content_id"] == peer["content_id"] and isinstance(signal, dict)):
                            continue
                        if signal.get("type") not in ("offer", "answer", "candidate"):
                            continue
                        state["signal_messages"] += 1
                        await emit(target, {"type": "signal", "from": peer_id, "signal": signal})
                except (ValueError, TypeError, KeyError):
                    continue
        finally:
            state["peers"].pop(peer_id, None)
            if peer["content_id"]:
                event = {"type": "left", "id": peer_id}
                for other in list(state["peers"].values()):
                    if other["content_id"] == peer["content_id"]:
                        await emit(other, event)
        return ws

    app.router.add_get("/", index)
    app.router.add_get("/client.js", client_js)
    app.router.add_get("/resumable_output.js", resumable_js)
    app.router.add_get("/range_reader.js", range_reader_js)
    app.router.add_get("/sha256_stream.js", sha256_js)
    app.router.add_get("/bounded_output.js", bounded_output_js)
    app.router.add_get("/style.css", css)
    app.router.add_get("/manifest.json", get_manifest)
    app.router.add_get("/stats", stats)
    app.router.add_route("GET", "/origin/demo.bin", origin)
    app.router.add_route("HEAD", "/origin/demo.bin", origin)
    app.router.add_get("/ws", websocket)
    return app


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--manifest", type=Path, default=HERE / "manifest.json")
    parser.add_argument("--demo-size", type=int, default=8 * 1024 * 1024)
    parser.add_argument("--demo-file", type=Path, default=HERE / "data/demo.bin")
    args = parser.parse_args()
    demo = args.demo_file
    if not args.manifest.exists():
        ensure_demo(demo, args.demo_size)
        generated = make_manifest(demo, "/origin/demo.bin")
        args.manifest.write_text(json.dumps(generated, indent=2) + "\n", encoding="utf-8")
    else:
        result = json.loads(args.manifest.read_text())
        if result["origin_url"] == "/origin/demo.bin" and not demo.exists():
            raise FileNotFoundError("Local /origin/demo.bin is missing; regenerate demo with --manifest from a valid file")
    app = create_app(args.manifest, demo if demo.exists() else None)
    print(f"Demo: http://{args.host}:{args.port}/ | source={app[STATE_KEY]['manifest']['origin_url']} | chunks={len(app[STATE_KEY]['manifest']['chunks'])}", flush=True)
    web.run_app(app, host=args.host, port=args.port, print=None)

if __name__ == "__main__":
    main()