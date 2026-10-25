"""Versioned synthetic HTML / embedded-JSON discovery fixtures.

No real Literotica markup is recognized. This fixture-only adapter is designed
for offline quality and page-order tests, and deliberately fails closed when
visible and embedded representations disagree.
"""
from __future__ import annotations

from html.parser import HTMLParser

from .model import DiscoveryPage, SchemaChanged, parse_fixture_page


class EmbeddedCatalogJSON(HTMLParser):
    """Reads a single <script type=application/json data-fixture-catalog=v1>."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.active = False
        self.seen = False
        self.closed = False
        self.found_main = False
        self.parts: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = dict(attrs)
        if tag == "main" and "data-fixture-kind" in attributes:
            self.found_main = True
        if tag == "script" and attributes.get("data-fixture-catalog") == "v1":
            if self.seen or attributes.get("type") != "application/json":
                raise SchemaChanged("duplicate or invalid embedded JSON fixture script")
            self.seen = True
            self.active = True

    def handle_data(self, data: str) -> None:
        if self.active:
            self.parts.append(data)

    def handle_endtag(self, tag: str) -> None:
        if tag == "script" and self.active:
            self.active = False
            self.closed = True


def parse_mixed_fixture_html(payload: bytes, *, expected_kind: str, requested_page: int, max_pages: int) -> DiscoveryPage:
    """Require a valid v1 view; when both views exist they must agree exactly."""
    if len(payload) > 2 * 1024 * 1024:
        raise SchemaChanged("synthetic mixed HTML exceeds 2 MiB")
    try:
        text = payload.decode("utf-8", "strict")
        reader = EmbeddedCatalogJSON()
        reader.feed(text)
        reader.close()
    except (UnicodeError, ValueError) as exc:
        raise SchemaChanged("invalid synthetic mixed HTML") from exc
    if not reader.found_main and not reader.seen:
        raise SchemaChanged("synthetic HTML contains no recognized fixture views")
    embedded = None
    if reader.seen:
        if not reader.closed:
            raise SchemaChanged("unfinished embedded JSON fixture")
        embedded = parse_fixture_page("".join(reader.parts).encode("utf-8"), expected_kind=expected_kind,
                                      requested_page=requested_page, max_pages=max_pages)
    visible = None
    if reader.found_main:
        # Lazy import breaks model.py <-> html_fixture.py cycle.
        from .model import _SyntheticHTMLReader
        visible = parse_fixture_page(_SyntheticHTMLReader().parse(payload), expected_kind=expected_kind,
                                     requested_page=requested_page, max_pages=max_pages)
    if visible is not None and embedded is not None and visible != embedded:
        raise SchemaChanged("embedded JSON disagrees with visible synthetic catalog")
    return embedded if embedded is not None else visible  # type: ignore[return-value]
