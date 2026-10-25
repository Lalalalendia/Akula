"""Offline parser for previously saved, permission-reviewed Literotica story HTML.

No network requests, browser automation, author login, pagination crawling or
site-wide completeness assertions. This module recognizes two page-content
layouts described by the published FanFicFare adapter; it MUST be validated
against a rights-cleared live snapshot corpus before production use.
"""
from __future__ import annotations

import re
from dataclasses import dataclass
from urllib.parse import parse_qs, urlsplit

from bs4 import BeautifulSoup


class SnapshotRejected(ValueError):
    """Do not silently publish when the page layout or pagination changes."""


# Domain is explicit: no redirects/hostname resemblance or credential-bearing URL.
# Language subdomains are treated as distinct source identities.
STORY_HOST = re.compile(r"(?:www|[a-z]{2,15})(?:\.i)?\.literotica\.com\Z", re.I)
SLUG = re.compile(r"[A-Za-z0-9_-]{1,150}\Z")
PAGE_WORDS = re.compile(r"\bPage\s+(\d{1,5})\s+of\s+(\d{1,5})\b", re.I)


@dataclass(frozen=True)
class StoryPageURL:
    host: str
    slug: str
    page: int


def story_url(url: str) -> StoryPageURL:
    if not isinstance(url, str) or len(url) > 1024:
        raise SnapshotRejected("invalid story URL")
    parsed = urlsplit(url)
    if (parsed.scheme != "https" or not parsed.hostname or not STORY_HOST.fullmatch(parsed.hostname)
            or parsed.username is not None or parsed.password is not None or parsed.port not in (None, 443)
            or parsed.fragment):
        raise SnapshotRejected("story URL not on approved source host")
    path = parsed.path.split("/")
    if len(path) != 3 or path[1] != "s" or not SLUG.fullmatch(path[2]):
        raise SnapshotRejected("not a story page URL")
    q = parse_qs(parsed.query, keep_blank_values=True, strict_parsing=True)
    if set(q) not in (set(), {"page"}):
        raise SnapshotRejected("unapproved story query parameters")
    if "page" not in q:
        page = 1
    else:
        value = q["page"]
        if len(value) != 1 or not value[0].isdigit() or value[0].startswith("0"):
            raise SnapshotRejected("invalid page query")
        page = int(value[0])
    if page > 3000 or page < 1:
        raise SnapshotRejected("story page number out of bounds")
    return StoryPageURL(parsed.hostname.lower(), path[2], page)


def series_url(url: str) -> tuple[str, str]:
    parsed = urlsplit(url)
    if (parsed.scheme != "https" or not parsed.hostname or not STORY_HOST.fullmatch(parsed.hostname)
            or parsed.query or parsed.fragment or parsed.username or parsed.password or parsed.port not in (None, 443)):
        raise SnapshotRejected("invalid series URL")
    path = parsed.path.split("/")
    if len(path) != 4 or path[1:3] != ["series", "se"] or not SLUG.fullmatch(path[3]):
        raise SnapshotRejected("invalid series path")
    return parsed.hostname.lower(), path[3]


@dataclass(frozen=True)
class ParsedStoryPage:
    text: str
    page: int
    expected_pages: int
    layout: str


def _paragraph_text(element) -> str:
    # Work on the copy that BeautifulSoup built. Explicitly preserve <br>.
    from bs4 import NavigableString
    fragments: list[str] = []
    for child in element.descendants:
        if getattr(child, "name", None) == "br":
            fragments.append("\n")
        elif isinstance(child, NavigableString) and child.parent.name not in ("script", "style"):
            fragments.append(str(child))
    text = "".join(fragments).replace("\xa0", " ")
    return "\n".join(" ".join(line.split()) for line in text.split("\n")).strip()


def parse_saved_story_page(raw_html: bytes, *, url: str, page: int, expected_pages: int) -> ParsedStoryPage:
    """Parse only saved bytes, requiring verifiable text area and pagination.

    The caller's selected catalog (not this parser) is authoritative for chapter
    identity/order. A successful parse certifies only ONE listed page.
    """
    info = story_url(url)
    if (type(page) is not int or type(expected_pages) is not int or
            not 1 <= page <= expected_pages <= 3000 or info.page != page):
        raise SnapshotRejected("page URL and selected catalog differ")
    if not isinstance(raw_html, bytes) or not 100 <= len(raw_html) <= 1024 * 1024:
        raise SnapshotRejected("invalid snapshot size")
    try:
        source = raw_html.decode("utf-8", "strict")
    except UnicodeDecodeError as exc:
        raise SnapshotRejected("invalid snapshot encoding") from exc
    doc = BeautifulSoup(source, "html.parser")
    if not (doc.html and doc.body):
        raise SnapshotRejected("unexpected or blocked HTML layout")
    if doc.select_one('input[type="password"], form[action*="login"], form[action*="captcha"]'):
        raise SnapshotRejected("login/challenge cannot be published")
    candidates = doc.select('div.aa_ht, div[class^="_article__content_"]')
    # One recognized content element only; unknown variants go to review.
    if len(candidates) != 1:
        raise SnapshotRejected("missing or ambiguous story content container")
    article = candidates[0]
    if article.select("script, iframe, form, img, svg, video, audio"):
        raise SnapshotRejected("non-text article cannot be silently reduced to text")
    paras = article.select("p")
    if not paras:
        raise SnapshotRejected("article contains no paragraph nodes")
    paragraphs = [_paragraph_text(p) for p in paras]
    paragraphs = [p for p in paragraphs if p]
    if not paragraphs:
        raise SnapshotRejected("article paragraphs are empty")
    body = "\n\n".join(paragraphs)
    if len(body.strip()) < 40 or len(body.encode("utf-8")) > 1024 * 1024 or "\x00" in body:
        raise SnapshotRejected("story body is empty, suspiciously small or oversized")
    # Distinguish a 7-page story from a 1-page story. The 2026 site has
    # pagination nav with a class containing _pagination_, but older HTML may
    # display 'Page N of M'; either must agree with approved catalog.
    navs = doc.select('nav[class*="_pagination_"]')
    if len(navs) > 1:
        raise SnapshotRejected("ambiguous page navigation")
    page_totals = []
    if navs:
        for link in navs[0].select("a[href]"):
            href = link.get("href", "")
            if "page=" in href:
                try:
                    q = parse_qs(urlsplit(href).query)
                    if "page" in q and len(q["page"]) == 1:
                        page_totals.append(int(q["page"][0]))
                except ValueError:
                    raise SnapshotRejected("broken pagination URL") from None
    visible_page_banner = PAGE_WORDS.findall(doc.get_text(" ", strip=True))
    if visible_page_banner:
        numbers = {(int(n), int(total)) for n, total in visible_page_banner}
        if numbers != {(page, expected_pages)}:
            raise SnapshotRejected("page number or total differs from approved catalog")
    elif expected_pages > 1:
        if not navs or max(page_totals, default=1) != expected_pages:
            raise SnapshotRejected("cannot prove full multipage story from saved snapshot")
    elif any(n > 1 for n in page_totals):
        raise SnapshotRejected("selected catalog missed additional story pages")
    kind = "legacy-aa_ht" if "aa_ht" in article.get("class", []) else "article-content-2026"
    return ParsedStoryPage(body, page, expected_pages, kind)
