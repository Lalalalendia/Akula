"""Contract test for the MinIO HTTPS gateway against a synthetic local S3 Range origin.

No Docker, AWS keys or real WebRTC are required. Browser E2E is tested separately.
"""
import asyncio
import hashlib
import sys
from pathlib import Path

from aiohttp import ClientSession, web

sys.path.insert(0, str(Path(__file__).parent))
import minio_tls_gateway as gateway


async def listen(app):
    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "127.0.0.1", 0)
    await site.start()
    return runner, f"http://127.0.0.1:{site._server.sockets[0].getsockname()[1]}"


async def exercise():
    data = bytes(range(256)) * 2048
    bucket = "akula-public-webrtc-contract"
    key = "immutable/8m.dat"
    etag = '"pinned-test-revision"'
    calls = []

    async def origin(request):
        calls.append((request.method, request.headers.get("Range")))
        if request.headers.get("If-Match") not in (None, etag):
            return web.Response(status=412, body=b"", headers={"ETag": etag})
        start, end, status = 0, len(data) - 1, 200
        if "Range" in request.headers:
            import re
            match = re.fullmatch(r"bytes=(\d+)-(\d*)", request.headers["Range"])
            if not match:
                return web.Response(status=416, headers={"Content-Range": f"bytes */{len(data)}"})
            start = int(match.group(1))
            end = int(match.group(2)) if match.group(2) else len(data) - 1
            if start >= len(data) or end < start or end >= len(data):
                return web.Response(status=416, headers={"Content-Range": f"bytes */{len(data)}"})
            status = 206
        payload = data[start:end + 1]
        headers = {"ETag": etag, "Accept-Ranges": "bytes",
                   "Content-Type": "application/octet-stream",
                   "Content-Length": str(len(payload))}
        if status == 206:
            headers["Content-Range"] = f"bytes {start}-{end}/{len(data)}"
        return web.Response(body=b"" if request.method == "HEAD" else payload,
                            status=status, headers=headers)

    source = web.Application()
    source.router.add_route("*", f"/{bucket}/{key}", origin)
    source_runner, source_url = await listen(source)
    old_endpoint = gateway.S3_ENDPOINT
    gateway.S3_ENDPOINT = source_url
    bridge_runner = None
    try:
        bridge_runner, url = await listen(gateway.make_app(bucket, key))
        async with ClientSession() as session:
            opts = {"headers": {"Origin": gateway.ALLOW_ORIGIN,
                                "Access-Control-Request-Headers": "range,if-match"}}
            async with session.options(f"{url}/{bucket}/{key}", **opts) as response:
                assert response.status == 204
                assert response.headers["Access-Control-Allow-Origin"] == gateway.ALLOW_ORIGIN
                assert "Range" in response.headers["Access-Control-Allow-Headers"]

            async with session.get(
                f"{url}/{bucket}/{key}",
                headers={"Range": "bytes=0-262143", "If-Match": etag,
                         "Origin": gateway.ALLOW_ORIGIN}
            ) as response:
                payload = await response.read()
                assert response.status == 206
                assert response.headers["Content-Range"] == f"bytes 0-262143/{len(data)}"
                assert response.headers["Content-Length"] == "262144"
                assert response.headers["ETag"] == etag
                assert "Content-Range" in response.headers["Access-Control-Expose-Headers"]
                assert payload == data[:262144]
                assert hashlib.sha256(payload).digest() == hashlib.sha256(data[:262144]).digest()

            async with session.head(f"{url}/{bucket}/{key}") as response:
                assert response.status == 200
                assert response.headers["Content-Length"] == str(len(data))
                assert await response.read() == b""

            async with session.get(f"{url}/{bucket}/{key}",
                                   headers={"If-Match": '"wrong-etag"'}) as response:
                assert response.status == 412
                assert await response.read() == b""

            async with session.get(f"{url}/{bucket}/{key}",
                                   headers={"Range": "bytes=999999-"}) as response:
                assert response.status == 416
                assert response.headers["Content-Range"] == f"bytes */{len(data)}"

            async with session.get(f"{url}/{bucket}/other-object") as response:
                assert response.status == 404
            async with session.post(f"{url}/{bucket}/{key}", data=b"bad") as response:
                assert response.status == 405

            async with session.get(f"{url}/metrics") as response:
                metrics = await response.json()
                assert metrics["origin_bytes"] == 262144
                assert metrics["range_gets"] == 2
                assert metrics["origin_gets"] == 3
        assert calls == [
            ("GET", "bytes=0-262143"), ("HEAD", None),
            ("GET", None), ("GET", "bytes=999999-")
        ]
    finally:
        gateway.S3_ENDPOINT = old_endpoint
        if bridge_runner:
            await bridge_runner.cleanup()
        await source_runner.cleanup()


def test_gateway_s3_range_cors_and_restricted_surface():
    asyncio.run(exercise())
