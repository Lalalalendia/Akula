"""Offline-first, fixture-only discovery engine; no website access."""
from .engine import DiscoveryEngine, DiscoveryStore, FixtureResponse, MemoryFixtureSource
from .objects import FileObjectSink, S3ObjectSink
from .model import canonical_fixture_url, recent_feed_gap
from .rescan import verify_partition_across_passes
from .publisher import WorkPublisher, Work, Chapter, ChapterPage, assemble
from .integration import publish_discovered_fixture

__all__ = [
    "DiscoveryEngine", "DiscoveryStore", "FixtureResponse", "MemoryFixtureSource",
    "FileObjectSink", "S3ObjectSink", "canonical_fixture_url", "recent_feed_gap", "verify_partition_across_passes",
    "WorkPublisher", "Work", "Chapter", "ChapterPage", "assemble", "publish_discovered_fixture",
]
