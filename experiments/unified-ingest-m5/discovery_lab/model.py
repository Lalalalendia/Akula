"""Strict typed discovery contract for synthetic fixtures only."""
from __future__ import annotations

import json
import re
from dataclasses import dataclass
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit


class SchemaChanged(ValueError):
    pass


_ENTITY_ID = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9_.:-]{0,100}$")
_ALLOWED_KINDS = frozenset({"author", "work", "series", "chapter", "category"})
_ALLOWED_DOC_KINDS = frozenset({"author_catalog", "category_alpha", "series_index", "recent_window"})


def require_entity_id(value: object) -> str:
    if not isinstance(value, str) or not _ENTITY_ID.fullmatch(value):
        raise SchemaChanged(f"invalid entity ID: {value!r}")
    return value


def canonical_fixture_url(url: str) -> str:
    """Offline fixture URLs are restricted to example.invalid; never fetch them."""
    parts = urlsplit(url)
    if parts.scheme != "https" or parts.hostname != "fixture.invalid" or parts.username or parts.password or parts.port:
        raise ValueError("fixture URL must have origin https://fixture.invalid")
    if not parts.path.startswith("/") or "//" in parts.path or ".." in parts.path:
        raise ValueError("invalid fixture path")
    valid = []
    for key, value in parse_qsl(parts.query, keep_blank_values=True):
        if key == "page":
            if not value.isascii() or not value.isdecimal() or not 1 <= int(value) <= 100_000:
                raise ValueError("bad page")
            valid.append(("page", str(int(value))))
        elif key in {"utm_source", "utm_medium", "utm_campaign"}:
            continue
        else:
            raise ValueError("unsupported query parameter")
    if len([1 for key, _ in valid if key == "page"]) > 1:
        raise ValueError("ambiguous page")
    return urlunsplit(("https", "fixture.invalid", parts.path.rstrip("/") or "/", urlencode(valid), ""))


def recent_feed_gap(previous_check_ts: float, next_check_ts: float, window_days: float = 7.0) -> bool:
    """A gap larger than the observed feed window requires separate backfill."""
    if next_check_ts < previous_check_ts or window_days <= 0:
        raise ValueError("invalid timestamps/window")
    return next_check_ts - previous_check_ts > window_days * 86_400


@dataclass(frozen=True)
class Relation:
    parent_kind: str
    parent_id: str
    rel: str
    ordinal: int | None


@dataclass(frozen=True)
class Item:
    kind: str
    entity_id: str
    title: str
    relations: tuple[Relation, ...]

    @property
    def key(self) -> str:
        return f"{self.kind}:{self.entity_id}"


@dataclass(frozen=True)
class DiscoveryPage:
    document_kind: str
    current_page: int
    last_page: int
    declared_total: int | None
    items: tuple[Item, ...]
    empty_valid: bool


def parse_fixture_page(payload: bytes, *, expected_kind: str, requested_page: int, max_pages: int) -> DiscoveryPage:
    """Deterministic parser for synthetic JSON; not a Literotica adapter."""
    if len(payload) > 2 * 1024 * 1024:
        raise SchemaChanged("fixture size exceeds 2 MiB bound")
    try:
        data = json.loads(payload.decode("utf-8", errors="strict"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise SchemaChanged("invalid UTF-8 JSON fixture") from exc
    if not isinstance(data, dict) or data.get("kind") not in _ALLOWED_DOC_KINDS:
        raise SchemaChanged("unrecognized page kind")
    if data["kind"] != expected_kind:
        raise SchemaChanged("wrong document kind")
    page, last = data.get("current_page"), data.get("last_page")
    if type(page) is not int or type(last) is not int or not 1 <= page <= last <= max_pages:
        raise SchemaChanged("invalid pagination bounds")
    if page != requested_page:
        raise SchemaChanged("incorrect current_page or repeated previous page")
    total = data.get("total_items")
    if total is not None and (type(total) is not int or total < 0 or total > 5_000_000):
        raise SchemaChanged("invalid declared_total")
    raw_items = data.get("items")
    if not isinstance(raw_items, list) or len(raw_items) > 5_000:
        raise SchemaChanged("invalid items field")
    if not raw_items and (not data.get("empty_valid") or total not in (None, 0)):
        raise SchemaChanged("suspicious empty 200 or nonempty declared catalog")
    items = []
    for raw in raw_items:
        if not isinstance(raw, dict) or raw.get("kind") not in _ALLOWED_KINDS:
            raise SchemaChanged("unrecognized item kind")
        if not isinstance(raw.get("title"), str) or not raw["title"].strip():
            raise SchemaChanged("missing item title")
        rels = []
        for rel in raw.get("relations", []):
            if not isinstance(rel, dict) or rel.get("parent_kind") not in _ALLOWED_KINDS:
                raise SchemaChanged("invalid relationship")
            ord_ = rel.get("ordinal")
            if ord_ is not None and (type(ord_) is not int or not 1 <= ord_ <= 1_000_000):
                raise SchemaChanged("invalid relationship ordinal")
            if rel.get("rel") not in ("series_chapter", "author_work", "category_work", "series_work"):
                raise SchemaChanged("unrecognized relationship type")
            rels.append(Relation(rel["parent_kind"], require_entity_id(rel.get("parent_id")), rel["rel"], ord_))
        items.append(Item(raw["kind"], require_entity_id(raw.get("id")), raw["title"].strip(), tuple(rels)))
    return DiscoveryPage(data["kind"], page, last, total, tuple(items), bool(data.get("empty_valid", False)))


def fixture_json(kind: str, page: int, last_page: int, items: list[dict], *, total: int | None = None,
                 empty_valid: bool = False) -> bytes:
    return json.dumps({"kind": kind, "current_page": page, "last_page": last_page, "total_items": total,
                       "items": items, "empty_valid": empty_valid}, ensure_ascii=False, sort_keys=True,
                      separators=(",", ":")).encode("utf-8")


class _SyntheticHTMLReader:
    """A strict fixture *contract*, not a Literotica DOM adapter."""
    def __init__(self) -> None:
        from html.parser import HTMLParser

        parent = self
        class Reader(HTMLParser):
            def __init__(self):
                super().__init__(convert_charrefs=True)

            def handle_starttag(self, tag, attributes):
                attrs = dict(attributes)
                if tag == "main" and "data-fixture-kind" in attrs:
                    if parent.attrs is not None:
                        raise SchemaChanged("multiple main fixtures")
                    parent.attrs = attrs
                    parent.in_main = True
                elif tag == "article" and parent.in_main:
                    if parent.current is not None:
                        raise SchemaChanged("nested fixture article")
                    parent.current = {"kind": attrs.get("data-kind"), "id": attrs.get("data-id"),
                                      "title": "", "relations": []}
                elif tag == "h3" and parent.current is not None:
                    parent.in_title = True
                elif tag == "span" and parent.current is not None and "data-rel" in attrs:
                    parent.current["relations"].append({"parent_kind": attrs.get("data-parent-kind"),
                                                         "parent_id": attrs.get("data-parent-id"),
                                                         "rel": attrs.get("data-rel"),
                                                         "ordinal": int(attrs["data-ordinal"]) if
                                                         attrs.get("data-ordinal", "").isdigit() else None})

            def handle_data(self, data):
                if parent.in_title and parent.current is not None:
                    parent.current["title"] += data

            def handle_endtag(self, tag):
                if tag == "h3":
                    parent.in_title = False
                elif tag == "article" and parent.current is not None:
                    parent.items.append(parent.current)
                    parent.current = None
                elif tag == "main" and parent.in_main:
                    parent.closed = True
                    parent.in_main = False

        self.attrs = None
        self.items = []
        self.current = None
        self.in_main = False
        self.in_title = False
        self.closed = False
        self.reader = Reader()

    def parse(self, payload: bytes) -> bytes:
        if len(payload) > 2 * 1024 * 1024:
            raise SchemaChanged("HTML fixture exceeds 2 MiB")
        try:
            self.reader.feed(payload.decode("utf-8", errors="strict"))
            self.reader.close()
        except (UnicodeDecodeError, ValueError) as exc:
            raise SchemaChanged("invalid HTML fixture") from exc
        if not self.attrs or not self.closed or self.current is not None:
            raise SchemaChanged("missing/unfinished fixture main document")
        try:
            current = int(self.attrs["data-current-page"])
            last = int(self.attrs["data-last-page"])
            total_value = self.attrs.get("data-total-items", "unknown")
            total = None if total_value == "unknown" else int(total_value)
        except (KeyError, ValueError) as exc:
            raise SchemaChanged("HTML fixture has malformed pagination") from exc
        return fixture_json(self.attrs["data-fixture-kind"], current, last, self.items,
                            total=total, empty_valid=self.attrs.get("data-empty-valid") == "true")


def parse_fixture_html(payload: bytes, *, expected_kind: str, requested_page: int, max_pages: int) -> DiscoveryPage:
    """M0: strict synthetic HTML adapter for author/category listings only."""
    from .html_fixture import parse_mixed_fixture_html
    return parse_mixed_fixture_html(payload, expected_kind=expected_kind,
                                    requested_page=requested_page, max_pages=max_pages)
