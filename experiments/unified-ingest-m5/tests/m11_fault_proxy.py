"""One-shot real HTTP/TCP fault proxy for disposable local MinIO CI.

Only binds loopback; copies signed request headers and bodies unchanged. Never logs
credentials or payload bytes. Not for production use.
"""
from __future__ import annotations

import http.client
import socket
import threading
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit


@dataclass
class Fault:
    method: str
    contains: str
    phase: str  # before_send, after_200, partial_get
    used: bool = False


class FaultProxy:
    def __init__(self, endpoint: str):
        target = urlsplit(endpoint)
        if target.scheme != "http" or target.hostname != "127.0.0.1" or not target.port:
            raise ValueError("fault proxy target must be a local HTTP service")
        self.host, self.port = target.hostname, target.port
        self.lock = threading.Lock()
        self.faults: list[Fault] = []
        self.calls: dict[str, int] = {}
        self.httpd: ThreadingHTTPServer | None = None
        self.thread: threading.Thread | None = None

    def arm(self, method: str, contains: str, phase: str):
        if method not in {"PUT", "POST", "GET"} or phase not in {"before_send", "after_200", "partial_get"}:
            raise ValueError("unsupported fault")
        if not contains or len(contains) > 256:
            raise ValueError("invalid match")
        fault = Fault(method, contains, phase)
        with self.lock:
            self.faults.append(fault)
        return fault

    def take(self, method: str, path: str):
        with self.lock:
            self.calls[method] = self.calls.get(method, 0) + 1
            for fault in self.faults:
                if not fault.used and fault.method == method and fault.contains in path:
                    fault.used = True
                    return fault
        return None

    def _handler(self):
        outer = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            server_version = "FaultProxy/1.0"

            def log_message(self, *_):
                return  # URLs can contain user-defined object identifiers.

            def _close_without_reply(self):
                self.close_connection = True
                try:
                    self.connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                self.connection.close()

            def _relay(self):
                fault = outer.take(self.command, self.path)
                if fault and fault.phase == "before_send":
                    return self._close_without_reply()
                count = self.headers.get("Content-Length")
                if count is None:
                    if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
                        raise ValueError("unexpected unsigned chunked transport in CI")
                    remaining = 0
                else:
                    remaining = int(count)
                    if remaining < 0 or remaining > 128 * 1024 * 1024:
                        raise ValueError("oversized HTTP payload")
                connection = http.client.HTTPConnection(outer.host, outer.port, timeout=30)
                try:
                    connection.putrequest(self.command, self.path, skip_host=True, skip_accept_encoding=True)
                    for key, value in self.headers.items():
                        if key.lower() not in ("connection", "proxy-connection"):
                            connection.putheader(key, value)
                    connection.putheader("Connection", "close")
                    connection.endheaders()
                    while remaining:
                        block = self.rfile.read(min(256 * 1024, remaining))
                        if not block:
                            raise ConnectionError("client disconnected during request")
                        connection.send(block)
                        remaining -= len(block)
                    response = connection.getresponse()
                    if fault and fault.phase == "after_200" and 200 <= response.status < 300:
                        # Backend has committed and begun its successful response.
                        # Make the acknowledgement inaccessible to the client.
                        response.read()
                        return self._close_without_reply()
                    self.send_response_only(response.status, response.reason)
                    for key, value in response.getheaders():
                        if key.lower() not in ("connection", "transfer-encoding", "keep-alive"):
                            self.send_header(key, value)
                    self.send_header("Connection", "close")
                    self.end_headers()
                    if fault and fault.phase == "partial_get" and response.status == 200:
                        self.wfile.write(response.read(16))
                        self.wfile.flush()
                        return self._close_without_reply()
                    while chunk := response.read(256 * 1024):
                        self.wfile.write(chunk)
                    self.wfile.flush()
                    self.close_connection = True
                except (BrokenPipeError, ConnectionError, OSError, http.client.HTTPException):
                    return self._close_without_reply()
                finally:
                    connection.close()

            do_PUT = do_POST = do_GET = do_HEAD = do_DELETE = _relay

        return Handler

    def __enter__(self):
        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.httpd.daemon_threads = True
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self.thread.start()
        return self

    @property
    def endpoint(self):
        if self.httpd is None:
            raise RuntimeError("proxy is not running")
        return f"http://127.0.0.1:{self.httpd.server_port}"

    def __exit__(self, *_):
        if self.httpd is not None:
            self.httpd.shutdown()
            self.httpd.server_close()
        if self.thread:
            self.thread.join(timeout=5)

