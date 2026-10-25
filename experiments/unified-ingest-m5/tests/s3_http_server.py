"""Tiny in-process S3-like HTTP fixture for AWS SDK integration tests.

Not MinIO, not a security implementation: it deliberately ignores SigV4 and
supports only the PUT/GET/HEAD/DELETE semantics needed for this lab. This checks
Boto3 HTTP and error serialization, not provider compliance.
"""
from __future__ import annotations

import hashlib
import uuid
import xml.etree.ElementTree as ET
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from threading import Lock, Thread
from urllib.parse import unquote, urlsplit, parse_qs


class S3TestState:
    def __init__(self) -> None:
        self.objects: dict[tuple[str, str], bytes] = {}
        self.lock = Lock()
        self.puts = 0
        self.gets = 0
        self.precondition_failures = 0
        self.fail_ack_after_put = False
        self.corrupt_readback = False
        self.mpu: dict[tuple[str, str, str], dict[int, tuple[bytes,str]]] = {}
        self.mpu_initiations = 0
        self.mpu_parts = 0
        self.mpu_completions = 0
        self.mpu_aborts = 0
        self.fail_mpu_part_once = False
        self.fail_mpu_part_ack_once = False
        self.fail_mpu_complete_ack_once = False
        self.ignore_mpu_conditional = False  # explicitly unsafe provider probe


class S3FixtureServer:
    def __init__(self) -> None:
        self.state = S3TestState()
        state = self.state

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_args) -> None:
                return

            def key(self) -> tuple[str, str]:
                parts = unquote(urlsplit(self.path).path).lstrip("/").split("/", 1)
                if len(parts) != 2 or not all(parts):
                    raise ValueError("malformed S3 path")
                return (parts[0], parts[1])

            def answer(self, code: int, body: bytes = b"", *, etag: str | None = None) -> None:
                self.send_response(code)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Content-Type", "application/xml" if code >= 400 else "application/octet-stream")
                if etag is not None:
                    self.send_header("ETag", etag)
                self.end_headers()
                if body:
                    try:
                        self.wfile.write(body)
                    except (BrokenPipeError, ConnectionResetError):
                        # Test client intentionally closed an over-limit response.
                        pass

            def error(self, code: int, name: str) -> None:
                self.answer(code, f"<Error><Code>{name}</Code><Message>{name}</Message></Error>".encode())

            def do_PUT(self) -> None:
                try:
                    key = self.key()
                    query = parse_qs(urlsplit(self.path).query, keep_blank_values=True)
                    content_length = int(self.headers["Content-Length"])
                    if content_length < 0 or content_length > 16 * 1024 * 1024:
                        raise ValueError("invalid length")
                    body = self.rfile.read(content_length)
                    if len(body) != content_length:
                        raise ValueError("incomplete body")
                except (ValueError, TypeError, KeyError):
                    self.error(400, "InvalidRequest")
                    return
                if 'uploadId' in query:
                    upload_id = query['uploadId'][0]
                    try:
                        number = int(query['partNumber'][0])
                    except (ValueError, IndexError, KeyError):
                        self.error(400, 'InvalidPart')
                        return
                    with state.lock:
                        parts = state.mpu.get((key[0],key[1],upload_id))
                        if parts is None:
                            self.error(404,'NoSuchUpload')
                            return
                        if state.fail_mpu_part_once:
                            state.fail_mpu_part_once = False
                            self.error(503,'SlowDown')
                            return
                        etag='"'+hashlib.md5(body).hexdigest()+'"'
                        parts[number] = (body,etag)
                        state.mpu_parts += 1
                        lost=state.fail_mpu_part_ack_once
                        state.fail_mpu_part_ack_once=False
                    if lost:
                        try: self.connection.shutdown(2)
                        except OSError: pass
                        self.close_connection=True
                        return
                    self.answer(200,etag=etag)
                    return
                with state.lock:
                    if key in state.objects and self.headers.get("If-None-Match") == "*":
                        state.precondition_failures += 1
                        self.error(412, "PreconditionFailed")
                        return
                    match = self.headers.get("If-Match")
                    old = state.objects.get(key)
                    if match is not None:
                        if old is None or match != '"' + hashlib.md5(old).hexdigest() + '"':
                            state.precondition_failures += 1
                            self.error(412, "PreconditionFailed")
                            return
                    elif old is not None:
                        # All overwrites must use CAS on the current ETag.
                        state.precondition_failures += 1
                        self.error(412, "PreconditionFailed")
                        return
                    state.objects[key] = body
                    state.puts += 1
                    lost = state.fail_ack_after_put
                    state.fail_ack_after_put = False
                if lost:
                    try:
                        self.connection.shutdown(2)
                    except OSError:
                        pass
                    self.close_connection = True
                    return
                self.answer(200, etag='"'+hashlib.md5(body).hexdigest()+'"')

            def do_POST(self) -> None:
                try:
                    key = self.key()
                    query = parse_qs(urlsplit(self.path).query,keep_blank_values=True)
                except ValueError:
                    self.error(400,'InvalidRequest')
                    return
                if 'uploads' in query:
                    ident = uuid.uuid4().hex
                    with state.lock:
                        state.mpu[(key[0],key[1],ident)] = {}
                        state.mpu_initiations += 1
                    xml = ('<InitiateMultipartUploadResult><Bucket>'+key[0]+'</Bucket><Key>'+key[1]+
                           '</Key><UploadId>'+ident+'</UploadId></InitiateMultipartUploadResult>').encode()
                    self.answer(200,xml)
                    return
                if 'uploadId' not in query:
                    self.error(400,'InvalidRequest')
                    return
                ident=query['uploadId'][0]
                try:
                    length=int(self.headers['Content-Length'])
                    root=ET.fromstring(self.rfile.read(length))
                    entries=[]
                    for part in root.iter():
                        if part.tag.split('}')[-1]!='Part':
                            continue
                        fields={child.tag.split('}')[-1]:child.text for child in part}
                        entries.append((int(fields['PartNumber']),fields['ETag']))
                except Exception:
                    self.error(400,'MalformedXML')
                    return
                with state.lock:
                    saved=state.mpu.get((key[0],key[1],ident))
                    if saved is None:
                        self.error(404,'NoSuchUpload')
                        return
                    if key in state.objects and self.headers.get('If-None-Match') == '*' and not state.ignore_mpu_conditional:
                        state.precondition_failures += 1
                        self.error(412,'PreconditionFailed')
                        return
                    if not entries or [n for n,_ in entries] != list(range(1,len(entries)+1)):
                        self.error(400,'InvalidPartOrder')
                        return
                    if any(n not in saved or saved[n][1] != etag for n,etag in entries):
                        self.error(400,'InvalidPart')
                        return
                    if any(len(saved[n][0]) < 5*1024*1024 for n,_ in entries[:-1]):
                        self.error(400,'EntityTooSmall')
                        return
                    data=b''.join(saved[n][0] for n,_ in entries)
                    state.objects[key] = data
                    del state.mpu[(key[0],key[1],ident)]
                    state.mpu_completions += 1
                    lost=state.fail_mpu_complete_ack_once
                    state.fail_mpu_complete_ack_once=False
                if lost:
                    try: self.connection.shutdown(2)
                    except OSError: pass
                    self.close_connection=True
                    return
                self.answer(200,('<CompleteMultipartUploadResult><Location>fixture</Location><Bucket>'+key[0]+
                                   '</Bucket><Key>'+key[1]+'</Key><ETag>"multipart"</ETag></CompleteMultipartUploadResult>').encode())

            def do_GET(self) -> None:
                try:
                    key = self.key()
                except ValueError:
                    self.error(400, "InvalidRequest")
                    return
                query=parse_qs(urlsplit(self.path).query,keep_blank_values=True)
                if 'uploadId' in query:
                    with state.lock:
                        parts=state.mpu.get((key[0],key[1],query['uploadId'][0]))
                        if parts is None:
                            self.error(404,'NoSuchUpload')
                            return
                        nodes=''.join('<Part><PartNumber>'+str(n)+'</PartNumber><ETag>'+etag+
                            '</ETag><Size>'+str(len(chunk))+'</Size></Part>'
                            for n,(chunk,etag) in sorted(parts.items()))
                    self.answer(200,('<ListPartsResult><Bucket>'+key[0]+'</Bucket><Key>'+key[1]+
                           '</Key><UploadId>'+query['uploadId'][0]+ '</UploadId><IsTruncated>false</IsTruncated>'+nodes+
                           '</ListPartsResult>').encode())
                    return
                with state.lock:
                    body = state.objects.get(key)
                    if body is not None:
                        state.gets += 1
                        if state.corrupt_readback:
                            body = body+b"corrupt"
                if body is None:
                    self.error(404, "NoSuchKey")
                else:
                    self.answer(200, body, etag='"'+hashlib.md5(body).hexdigest()+'"')

            def do_HEAD(self) -> None:
                try:
                    key = self.key()
                except ValueError:
                    self.answer(400)
                    return
                with state.lock:
                    body = state.objects.get(key)
                self.answer(200 if body is not None else 404,
                            etag='"'+hashlib.md5(body).hexdigest()+'"' if body is not None else None)

            def do_DELETE(self) -> None:
                try:
                    key = self.key()
                except ValueError:
                    self.error(400, "InvalidRequest")
                    return
                query=parse_qs(urlsplit(self.path).query,keep_blank_values=True)
                with state.lock:
                    if 'uploadId' in query:
                        found=state.mpu.pop((key[0],key[1],query['uploadId'][0]),None)
                        if found is None:
                            self.error(404,'NoSuchUpload')
                            return
                        state.mpu_aborts+=1
                    else:
                        state.objects.pop(key, None)
                self.answer(204)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread: Thread | None = None

    @property
    def endpoint(self) -> str:
        return f"http://127.0.0.1:{self.server.server_port}"

    def __enter__(self):
        self.thread = Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        return self

    def __exit__(self, *_args) -> None:
        self.server.shutdown()
        self.server.server_close()
        if self.thread is not None:
            self.thread.join(timeout=5)
