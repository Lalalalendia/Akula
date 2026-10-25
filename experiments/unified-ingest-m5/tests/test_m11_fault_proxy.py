"""Real loopback-socket tests of the deterministic fault proxy (no Docker)."""
from __future__ import annotations

import http.client
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit
from m11_fault_proxy import FaultProxy


class Backend(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    calls = 0
    stored = b""

    def log_message(self, *_):
        return

    def do_PUT(self):
        body = self.rfile.read(int(self.headers["Content-Length"]))
        type(self).stored = body
        type(self).calls += 1
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"OK")

    def do_GET(self):
        body = type(self).stored
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class NetworkProxyTests(unittest.TestCase):
    def setUp(self):
        Backend.calls, Backend.stored = 0, b""
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Backend)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.proxy = FaultProxy(f"http://127.0.0.1:{self.server.server_port}")
        self.proxy.__enter__()
        self.addCleanup(self.cleanup)

    def cleanup(self):
        self.proxy.__exit__(None,None,None)
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def request(self, method, path, body=None):
        ep = urlsplit(self.proxy.endpoint)
        c = http.client.HTTPConnection(ep.hostname, ep.port, timeout=3)
        try:
            c.request(method, path, body=body)
            r = c.getresponse()
            return r.status, r.read()
        finally:
            c.close()

    def test_put_ack_lost_after_backend_commit_and_readback_succeeds(self):
        f = self.proxy.arm("PUT", "ack-lost", "after_200")
        with self.assertRaises((http.client.RemoteDisconnected, OSError, ConnectionError)):
            self.request("PUT", "/bucket/ack-lost", b"saved")
        self.assertTrue(f.used)
        self.assertEqual(Backend.calls, 1)
        self.assertEqual(self.request("GET", "/bucket/ack-lost"), (200, b"saved"))

    def test_drop_before_send_does_not_commit_and_can_retry(self):
        f = self.proxy.arm("PUT", "drop-before", "before_send")
        with self.assertRaises((http.client.RemoteDisconnected, OSError, ConnectionError)):
            self.request("PUT", "/bucket/drop-before", b"saved")
        self.assertTrue(f.used)
        self.assertEqual(Backend.calls, 0)
        self.assertEqual(self.request("PUT", "/bucket/drop-before", b"saved"), (200, b"OK"))
        self.assertEqual(Backend.calls, 1)

