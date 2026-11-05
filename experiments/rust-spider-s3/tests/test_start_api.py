"""Guard Scrapy 2.19 start API without importing optional scrapy locally."""
import ast
from pathlib import Path
import unittest


class TestStartAPI(unittest.TestCase):
    def test_scrapy_start_is_async_generator(self):
        source = Path(__file__).resolve().parents[1] / "python_crawler.py"
        module = ast.parse(source.read_text(encoding="utf-8"))
        spider = next(node for node in module.body if isinstance(node, ast.ClassDef) and node.name == "TrialSpider")
        starts = [node for node in spider.body if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name in ("start", "start_requests")]
        self.assertEqual([node.name for node in starts], ["start"])
        self.assertIsInstance(starts[0], ast.AsyncFunctionDef)
        self.assertTrue(any(isinstance(n, ast.Yield) for n in ast.walk(starts[0])))
