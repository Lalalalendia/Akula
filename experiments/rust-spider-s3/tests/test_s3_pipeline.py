"""Validate S3 idempotence and capped concurrency without live Scrapy."""
import asyncio
import importlib.util
import os
import sys
import threading
import time
import types
import unittest
from pathlib import Path
from unittest.mock import patch

class FakeS3:
    def __init__(self):
        self.items = {}
        self.ops = []

    def list_objects_v2(self, *, Bucket, Prefix, MaxKeys):
        self.ops.append(("list", Prefix))
        return {"Contents": [{"Key": k} for k in self.items if k.startswith(Prefix)][:MaxKeys]}

    def head_object(self, *, Bucket, Key):
        self.ops.append(("head", Key))
        return {"Metadata": self.items[Key]["Metadata"]}

    def put_object(self, *, Bucket, Key, Body, Metadata, ContentType):
        self.ops.append(("put", Key))
        self.items[Key] = {"Body": Body, "Metadata": Metadata}

class TestS3Pipeline(unittest.TestCase):
    def setUp(self):
        import boto3
        self.fake = FakeS3()
        fake_scrapy = types.ModuleType("scrapy")
        fake_scrapy.Spider = type("Spider", (), {})
        fake_crawler = types.ModuleType("scrapy.crawler")
        fake_crawler.CrawlerProcess = type("CrawlerProcess", (), {})
        env = {"S3_ENDPOINT": "http://localhost:9000", "S3_BUCKET": "test", "S3_PREFIX": "probe", "TRIAL_ROOT": "http://localhost:8080/index.html", "CRASH_AFTER_PUT_N": "0"}
        path = Path(__file__).resolve().parents[1] / "python_crawler.py"
        spec = importlib.util.spec_from_file_location("trial_s3_test_client", path)
        mod = importlib.util.module_from_spec(spec)
        with patch.dict(os.environ, env), patch.dict(sys.modules, {"scrapy": fake_scrapy, "scrapy.crawler": fake_crawler}):
            spec.loader.exec_module(mod)
        with patch("boto3.client", return_value=self.fake):
            self.pipeline = mod.S3Pipeline()

    def test_repeat_is_skip_not_second_put(self):
        x = {"name": "00001.html", "body": b"abc"}
        self.pipeline.upload(x)
        self.pipeline.upload(x)
        self.assertEqual([op[0] for op in self.fake.ops], ["list", "put", "list", "head"])

    def test_conflicting_bytes_fail_closed(self):
        self.pipeline.upload({"name": "00002.html", "body": b"a"})
        with self.assertRaisesRegex(RuntimeError, "different SHA-256"):
            self.pipeline.upload({"name": "00002.html", "body": b"b"})
        self.assertEqual([op[0] for op in self.fake.ops].count("put"), 1)

    def test_bounded_uploads(self):
        class Slow(FakeS3):
            def __init__(self):
                super().__init__()
                self.active = 0
                self.maximum = 0
                self.lock = threading.Lock()
            def put_object(self, **kw):
                with self.lock:
                    self.active += 1
                    self.maximum = max(self.maximum, self.active)
                try:
                    time.sleep(.02)
                    return super().put_object(**kw)
                finally:
                    with self.lock:
                        self.active -= 1
        slow = Slow()
        self.pipeline.client = slow
        async def run():
            await asyncio.gather(*(self.pipeline.process_item({"name": f"{n:05d}.html", "body": str(n).encode()}) for n in range(12)))
        asyncio.run(run())
        self.assertEqual(len(slow.items), 12)
        self.assertLessEqual(slow.maximum, 4)
        self.assertGreaterEqual(slow.maximum, 2)

if __name__ == "__main__":
    unittest.main()
