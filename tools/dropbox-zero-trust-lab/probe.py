#!/usr/bin/env python3
"""Opt-in Dropbox API conformance probe for Akula Drop Zero Trust.

Writes ONLY randomized synthetic objects in a separately authorized disposable
Dropbox App folder. Does not prove a malicious provider honest. Does not delete
objects. No GitHub Actions workflow or automatic CI invocation.
"""

import argparse
import hashlib
import json
import os
import secrets
import sys
import time
from dataclasses import dataclass
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

API_HOST = "api.dropboxapi.com"
CONTENT_HOST = "content.dropboxapi.com"
MAX_JSON = 65536
LIMIT = 8 * 1024 * 1024


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        raise RuntimeError("refused unexpected redirect")


OPENER = build_opener(NoRedirect)


@dataclass
class Reply:
    code: int
    data: bytes
    headers: dict


def checked_https(url: str, allowed_host: str, required_path: str):
    p = urlsplit(url)
    if (p.scheme != "https" or p.hostname != allowed_host or p.port not in (None, 443)
            or p.username is not None or p.password is not None
            or not p.path.startswith(required_path)):
        raise ValueError("untrusted URL or provider redirect refused")


def request(url: str, data: bytes, headers: dict, host: str, path: str) -> Reply:
    checked_https(url, host, path)
    req = Request(url, data=data, headers=headers, method="POST")
    try:
        with OPENER.open(req, timeout=45) as response:
            body = response.read(MAX_JSON + 1) if path != "/2/files/download" else response.read(LIMIT + 1)
            if len(body) > (LIMIT if path == "/2/files/download" else MAX_JSON):
                raise RuntimeError("bounded response size exceeded")
            return Reply(response.status, body, dict(response.headers))
    except HTTPError as error:
        with error:
            return Reply(error.code, error.read(MAX_JSON + 1), dict(error.headers))


def api(token: str, route: str, payload: dict) -> Reply:
    body = json.dumps(payload, separators=(",", ":"), ensure_ascii=True).encode("ascii")
    return request("https://" + API_HOST + route, body,
                   {"Authorization": "Bearer " + token,
                    "Content-Type": "application/json"}, API_HOST, route)


def issue(token: str, path: str) -> str:
    reply = api(token, "/2/files/get_temporary_upload_link", {
        "commit_info": {"path": path, "mode": "add", "autorename": False,
                        "strict_conflict": True}, "duration": 600})
    if reply.code != 200:
        raise RuntimeError("temporary link issuance failed HTTP " + str(reply.code))
    url = json.loads(reply.data)["link"]
    checked_https(url, CONTENT_HOST, "/apitul/")
    return url


def post_object(url: str, payload: bytes) -> Reply:
    return request(url, payload,
                   {"Content-Type": "application/octet-stream"},
                   CONTENT_HOST, "/apitul/")


def metadata(token: str, path: str) -> dict:
    # No blind write retries: these are owner-authenticated read-only polls.
    for pause in (0, 1, 2, 4):
        if pause:
            time.sleep(pause)
        r = api(token, "/2/files/get_metadata", {"path": path})
        if r.code == 200:
            value = json.loads(r.data)
            if value.get(".tag") != "file":
                raise RuntimeError("unexpected non-file remote object")
            return value
        if r.code != 409:
            raise RuntimeError("metadata failed HTTP " + str(r.code))
    raise RuntimeError("file absent after bounded read-only reconcile")


def download_revision(token: str, rev: str, expected_max: int) -> bytes:
    if not rev or len(rev) > 128 or not all(ch in "0123456789abcdef" for ch in rev):
        raise RuntimeError("untrusted revision syntax")
    url = "https://" + CONTENT_HOST + "/2/files/download"
    arg = json.dumps({"path": "rev:" + rev}, separators=(",", ":"))
    r = request(url, b"", {"Authorization": "Bearer " + token,
                           "Dropbox-API-Arg": arg}, CONTENT_HOST, "/2/files/download")
    if r.code != 200:
        raise RuntimeError("revision download HTTP " + str(r.code))
    if len(r.data) > expected_max:
        raise RuntimeError("readback exceeded the agreed size")
    return r.data


def dropbox_content_hash(data: bytes) -> str:
    blocks = (hashlib.sha256(data[i:i + 4 * 1024 * 1024]).digest()
              for i in range(0, len(data), 4 * 1024 * 1024))
    return hashlib.sha256(b"".join(blocks)).hexdigest()


def readback(token: str, path: str, expected: bytes) -> dict:
    meta = metadata(token, path)
    if meta.get("size") != len(expected):
        raise RuntimeError("remote size mismatch")
    if meta.get("content_hash") != dropbox_content_hash(expected):
        raise RuntimeError("Dropbox tree hash mismatch")
    payload = download_revision(token, meta["rev"], len(expected))
    if payload != expected or dropbox_content_hash(payload) != meta["content_hash"]:
        raise RuntimeError("exact revision readback mismatch")
    return {"file_id": meta.get("id"), "rev": meta["rev"],
            "bytes": meta["size"], "tree_hash": meta["content_hash"]}


def sibling_names(token: str, path: str) -> list:
    """Detect an unexpected auto-renamed second object inside the test App folder."""
    marker = path.rsplit("/", 1)[-1].rsplit(".", 1)[0].casefold()
    route, payload = "/2/files/list_folder", {"path": "", "recursive": False, "limit": 1000}
    matches = []
    for _ in range(5):
        r = api(token, route, payload)
        if r.code != 200:
            raise RuntimeError("list_folder HTTP " + str(r.code))
        value = json.loads(r.data)
        matches += [entry.get("name") for entry in value.get("entries", [])
                    if entry.get(".tag") == "file"
                    and str(entry.get("name", "")).casefold().startswith(marker)]
        if not value.get("has_more"):
            return matches
        route, payload = "/2/files/list_folder/continue", {"cursor": value["cursor"]}
    raise RuntimeError("test folder listing exceeded bounded five-page scan")


def p01_two_tickets(token: str, path: str) -> dict:
    a, b = secrets.token_bytes(128), secrets.token_bytes(129)
    link1 = issue(token, path)
    link2 = issue(token, path)
    first = post_object(link1, a)
    if first.code != 200:
        return {"status": "HOLD", "reason": "first upload HTTP " + str(first.code)}
    before = readback(token, path, a)
    second = post_object(link2, b)
    try:
        after = readback(token, path, a)
    except RuntimeError:
        return {"status": "FAIL", "second_http": second.code,
                "reason": "original exact revision was changed, lost, or unverifiable"}
    names = sibling_names(token, path)
    expected_name = path.rsplit("/", 1)[-1]
    only_one = len(names) == 1 and names[0].casefold() == expected_name.casefold()
    return {"status": "PASS" if second.code in (409, 410) and before == after and only_one else "FAIL",
            "second_http": second.code, "first_rev_stable": before == after,
            "no_renamed_sibling": only_one,
            "remote_size": after["bytes"], "file_id": after["file_id"], "rev": after["rev"]}


def p02_size_cap(token: str, path: str) -> dict:
    payload = secrets.token_bytes(LIMIT + 1)
    status = post_object(issue(token, path), payload).code
    if status == 200:
        meta = metadata(token, path)
        return {"status": "NO_8M_SERVER_CAP_OBSERVED" if meta.get("size") == len(payload)
                else "HOLD", "upload_http": status,
                "remote_size": meta.get("size"), "agreed_max": LIMIT}
    return {"status": "HOLD", "upload_http": status, "agreed_max": LIMIT,
            "note": "Rejection might be generic account/provider limit, not per-grant size enforcement"}


def p03_lost_ack(token: str, path: str) -> dict:
    payload = secrets.token_bytes(1024)
    # Deliberately discard the application's knowledge of the POST result.
    # This is an app-layer ACK-loss fault, NOT a simulated TCP reset.
    post_object(issue(token, path), payload)
    verified = readback(token, path, payload)
    return {"status": "PASS_RECONCILED_CLOUD_ONLY", "bytes": verified["bytes"],
            "file_id": verified["file_id"], "rev": verified["rev"],
            "app_layer_ack_discarded": True, "blind_retry": False,
            "durable_inbox_receipt": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true",
                        help="Explicitly enable disposable Dropbox writes")
    args = parser.parse_args()
    if not args.run:
        print("NOT RUN. Requires --run and disposable App-folder test credentials.")
        return 0
    token = os.getenv("AKULA_DROPBOX_TEST_TOKEN", "")
    expected = os.getenv("AKULA_DROPBOX_TEST_ACCOUNT_ID", "")
    confirm = os.getenv("AKULA_DROPBOX_DISPOSABLE_CONFIRM", "")
    if not token or not expected.startswith("dbid:") or confirm != "YES_DISPOSABLE_APP_FOLDER":
        print("HOLD: disposable token, expected account and explicit scope confirmation missing")
        return 2
    account = api(token, "/2/users/get_current_account", {})
    if account.code != 200 or json.loads(account.data).get("account_id") != expected:
        print("HOLD: Dropbox test account mismatch or missing account_info.read scope")
        return 2
    runid = secrets.token_hex(12)
    funcs = [("P01", p01_two_tickets), ("P02", p02_size_cap), ("P03", p03_lost_ack)]
    results = {}
    for name, fn in funcs:
        try:
            results[name] = fn(token, "/akula_zt_lab_" + runid + "_" + name.lower() + ".bin")
        except Exception as error:
            results[name] = {"status": "HOLD", "error_type": type(error).__name__}
    print(json.dumps({"kind": "AKULA_DROPBOX_ZT_P01_P03", "experiment": runid,
                      "account_checked": True, "external_provider_trust": "UNTRUSTED",
                      "results": results, "cleanup": "manual, disposable App folder only"},
                     sort_keys=True, indent=2))
    # No automatic deletions or risky retries. FAIL/HOLD is deliberate.
    return 0 if all(r["status"].startswith("PASS") for r in results.values()) else 1


if __name__ == "__main__":
    sys.exit(main())