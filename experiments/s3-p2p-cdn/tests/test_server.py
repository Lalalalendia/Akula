import asyncio
import hashlib
import json
import tempfile
from pathlib import Path

from aiohttp import ClientSession, web
from create_manifest import make_manifest
from server import create_app


def test_manifest_chunk_hashes_and_versions():
    with tempfile.TemporaryDirectory() as dir_name:
        f = Path(dir_name) / 'sample.bin'
        f.write_bytes(bytes(range(256)) * 7 + b'end')
        a = make_manifest(f, '/origin/demo.bin', chunk_size=1024)
        assert a['size'] == len(f.read_bytes())
        assert len(a['chunks']) == 2
        assert a['content_id'] == hashlib.sha256(f.read_bytes()).hexdigest()
        assert a['chunks'][0]['sha256'] == hashlib.sha256(f.read_bytes()[:1024]).hexdigest()
        assert a['chunks'][1]['sha256'] == hashlib.sha256(f.read_bytes()[1024:]).hexdigest()
        f.write_bytes(f.read_bytes() + b'changed')
        b = make_manifest(f, '/origin/demo.bin', chunk_size=1024)
        assert b['content_id'] != a['content_id']


def test_manifest_rejects_unsafe_url():
    with tempfile.TemporaryDirectory() as dir_name:
        f = Path(dir_name) / 'sample.bin'
        f.write_bytes(b'a' * 1024)
        for url in ('http://somewhere.com/object', 'file:///etc/passwd', 'ftp://host/file'):
            try:
                make_manifest(f, url)
            except ValueError:
                pass
            else:
                raise AssertionError(f'Accepted URL: {url}')


async def _aiohttp_workflow():
    with tempfile.TemporaryDirectory() as dir_name:
        p = Path(dir_name)
        source = p / 'source.bin'
        data = bytes(range(256)) * 64
        source.write_bytes(data)
        manifest = make_manifest(source, '/origin/demo.bin', chunk_size=1024)
        mf = p / 'manifest.json'
        mf.write_text(json.dumps(manifest), encoding='utf-8')
        app = create_app(mf, source)
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, '127.0.0.1', 0)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        try:
            async with ClientSession() as session:
                async with session.get(f'{base}/manifest.json') as r:
                    assert r.status == 200
                    assert (await r.json())['content_id'] == manifest['content_id']
                async with session.get(f'{base}/origin/demo.bin', headers={'Range': 'bytes=100-105'}) as r:
                    assert r.status == 206
                    assert r.headers['Content-Range'] == f'bytes 100-105/{len(data)}'
                    assert await r.read() == data[100:106]
                async with session.get(f'{base}/origin/demo.bin', headers={'Range': 'bytes=16384-'}) as r:
                    assert r.status == 416
                async with session.get(f'{base}/origin/demo.bin', headers={'Range': 'bytes=500-10'}) as r:
                    assert r.status == 416
                async with session.get(f'{base}/stats') as r:
                    stats = await r.json()
                    assert stats['origin_requests'] == 1
                    assert stats['origin_bytes'] == 6

                a = await session.ws_connect(base.replace('http:', 'ws:') + '/ws')
                b = await session.ws_connect(base.replace('http:', 'ws:') + '/ws')
                aid = (await a.receive_json())['id']
                bid = (await b.receive_json())['id']
                await a.send_json({'type': 'join', 'content_id': manifest['content_id']})
                assert (await a.receive_json())['type'] == 'joined'
                await b.send_json({'type': 'join', 'content_id': manifest['content_id']})
                assert (await b.receive_json())['peers'] == []
                await a.send_json({'type': 'announce', 'share': True, 'have': [0, 1]})
                av = await b.receive_json()
                assert av == {'type': 'availability', 'id': aid, 'have': [0, 1]}
                await b.send_json({'type': 'signal', 'to': aid, 'signal': {'type': 'offer', 'sdp': 'safe-test'}})
                sent = await a.receive_json()
                assert sent['type'] == 'signal' and sent['from'] == bid
                assert sent['signal']['sdp'] == 'safe-test'
                # Upload consent revoked: all advertised parts disappear immediately.
                await a.send_json({'type': 'announce', 'share': False, 'have': [0, 1]})
                assert (await b.receive_json())['have'] == []
                await a.close()
                assert (await b.receive_json())['type'] == 'left'
                await b.close()
        finally:
            await runner.cleanup()


def test_http_range_and_websocket_tracker():
    asyncio.run(_aiohttp_workflow())