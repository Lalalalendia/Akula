#!/usr/bin/env python3
"""TEST-ONLY loopback Bearer adapter for the official MCP conformance CLI.

This is not a production proxy and not suitable for testing streaming,
auth rejection, chunked framing, or DNS rebinding. The real Akula Core and
Bearer gate remain active behind the adapter.
"""
import argparse
import http.client
import os
import pathlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MAX_BODY = 4 * 1024 * 1024
HOP = {
    "authorization", "host", "connection", "proxy-connection",
    "keep-alive", "transfer-encoding", "content-length", "te",
    "trailer", "upgrade",
}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, format, *args):
        # Do not print tokens, headers or note contents into CI logs.
        return

    def relay(self):
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length < 0 or length > MAX_BODY:
                self.send_error(413)
                return
            if self.headers.get("Transfer-Encoding"):
                self.send_error(501, "test adapter does not handle chunked request bodies")
                return
            payload = self.rfile.read(length)
            upstream = http.client.HTTPConnection("127.0.0.1", self.server.upstream, timeout=12)
            headers = {
                key: value for key, value in self.headers.items()
                if key.lower() not in HOP
            }
            headers["Authorization"] = "Bearer " + self.server.bearer
            headers["Host"] = "127.0.0.1:" + str(self.server.upstream)
            headers["Connection"] = "close"
            headers["Content-Length"] = str(len(payload))
            upstream.request(self.command, self.path, body=payload, headers=headers)
            reply = upstream.getresponse()
            data = reply.read()
            self.send_response_only(reply.status)
            for key, value in reply.getheaders():
                if key.lower() not in HOP:
                    self.send_header(key, value)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Connection", "close")
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(data)
            upstream.close()
        except (OSError, ValueError, http.client.HTTPException) as exc:
            self.send_error(502, "Local fixture upstream error: " + type(exc).__name__)
        finally:
            self.close_connection = True

    do_GET = relay
    do_POST = relay
    do_DELETE = relay


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--upstream-port-file", type=pathlib.Path, required=True)
    parser.add_argument("--listen-port-file", type=pathlib.Path, required=True)
    args = parser.parse_args()
    bearer = os.environ["AKAI_MCP_TEST_BEARER"]
    upstream = int(args.upstream_port_file.read_text().strip())
    if not 1 <= upstream <= 65535 or len(bearer) < 32:
        raise ValueError("Invalid test fixture port or bearer")
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.daemon_threads = True
    server.upstream = upstream
    server.bearer = bearer
    args.listen_port_file.write_text(str(server.server_address[1]))
    server.serve_forever(poll_interval=0.1)


if __name__ == "__main__":
    main()
