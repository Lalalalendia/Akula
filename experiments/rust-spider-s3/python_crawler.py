"""Scrapy to signed S3 PutObject baseline (same concurrency limits as Rust trial)."""
import asyncio
import hashlib
import os
import threading

import boto3
from botocore.client import Config
import scrapy
from scrapy.crawler import CrawlerProcess

ENDPOINT = os.environ["S3_ENDPOINT"]
BUCKET = os.environ["S3_BUCKET"]
PREFIX = os.environ["S3_PREFIX"]
ROOT = os.environ["TRIAL_ROOT"]
CRASH_AFTER = int(os.environ.get("CRASH_AFTER_PUT_N", "0"))


class S3Pipeline:
    def __init__(self):
        self.client = boto3.client("s3", endpoint_url=ENDPOINT, region_name="us-east-1",
                                   config=Config(s3={"addressing_style": "path"}, max_pool_connections=4))
        self.n = 0
        self.lock = threading.Lock()
        self.s3_slots = asyncio.Semaphore(4)

    def upload(self, item):
        name = item["name"]
        body = item["body"]
        sha = hashlib.sha256(body).hexdigest()
        key = f"{PREFIX}/{name}"
        # Match the Rust client's ListObjectsV2 + conditional HeadObject sequence.
        listed = self.client.list_objects_v2(Bucket=BUCKET, Prefix=key, MaxKeys=2)
        if any(obj.get("Key") == key for obj in listed.get("Contents", [])):
            existing = self.client.head_object(Bucket=BUCKET, Key=key)
            if existing.get("Metadata", {}).get("sha256") != sha:
                raise RuntimeError(f"object has different SHA-256 metadata: {key}")
            print(f"SKIP {key} {sha}", flush=True)
            return
        self.client.put_object(Bucket=BUCKET, Key=key, Body=body, Metadata={"sha256": sha},
                               ContentType="text/html; charset=utf-8")
        with self.lock:
            self.n += 1
            n = self.n
        print(f"PUT {key} {sha} {n}", flush=True)
        if CRASH_AFTER and n >= CRASH_AFTER:
            # Failure between remote ACK and any local journal commit.
            os._exit(87)

    async def process_item(self, item, spider=None):
        async with self.s3_slots:
            await asyncio.to_thread(self.upload, item)
        return item


class TrialSpider(scrapy.Spider):
    name = "trial_s3"
    custom_settings = {
        "ITEM_PIPELINES": {f"{__name__}.S3Pipeline": 100},
        "CONCURRENT_REQUESTS": 4,
        "CONCURRENT_REQUESTS_PER_DOMAIN": 4,
        "CONCURRENT_ITEMS": 8,
        "ROBOTSTXT_OBEY": False,
        "LOG_LEVEL": "INFO",
        "RETRY_TIMES": 0,
        "DOWNLOAD_TIMEOUT": 20,
        "TWISTED_REACTOR": "twisted.internet.asyncioreactor.AsyncioSelectorReactor",
    }

    async def start(self):
        # Scrapy 2.13+ uses async start(); overriding start_requests is insufficient.
        yield scrapy.Request(ROOT, callback=self.parse, dont_filter=True)

    def parse(self, response):
        if response.url.endswith("/index.html"):
            for link in response.css("a::attr(href)").getall():
                yield response.follow(link, callback=self.parse)
            return
        if "/pages/" not in response.url:
            return
        if response.status != 200:
            raise RuntimeError(f"non-200 response: {response.url}")
        yield {"name": response.url.rsplit("/", 1)[-1], "body": response.body}


if __name__ == "__main__":
    process = CrawlerProcess(settings={"LOG_LEVEL": "INFO"})
    crawler = process.create_crawler(TrialSpider)
    process.crawl(crawler)
    process.start()
    scraped = int(crawler.stats.get_value("item_scraped_count", 0))
    expected = int(os.environ.get("TRIAL_PAGES", "160"))
    print(f"SCRAPY_STATS scraped={scraped} expected={expected}", flush=True)
    if scraped != expected:
        raise RuntimeError(f"Scrapy did not produce exactly {expected} items (got {scraped})")
    print("FINISHED python Scrapy crawl", flush=True)
