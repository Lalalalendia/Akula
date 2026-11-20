#!/usr/bin/env python3
"""Synthetic 01s negative contracts; no downloaded EXE is executed."""
import importlib.util
import unittest
import zlib
from pathlib import Path

HERE=Path(__file__).resolve().parent
S=importlib.util.spec_from_file_location("video_tools_01s",HERE/"video-tools-01s-notice-reconciliation.py")
M=importlib.util.module_from_spec(S)
S.loader.exec_module(M)

def mock_image(note,oversize=False):
    import struct
    raw=zlib.compress(note)
    name=b"THIRD_PARTY_LICENSES.txt\0"
    n=M.R.ENTRY.size+len(name)
    name+=b"\0"*((-n)%16)
    entry=M.R.ENTRY.pack(M.R.ENTRY.size+len(name),0,len(raw),
                         M.MAX_NOTICE+1 if oversize else len(note),1,b"b")+name
    cookie=M.R.COOKIE.pack(M.R.MAGIC,len(raw)+len(entry)+M.R.COOKIE.size,
                            len(raw),len(entry),314,b"python314.dll".ljust(64,b"\0"))
    return b"MZ"+b"\0"*1022+raw+entry+cookie

def inventory():
    modules=(["yt_dlp.extractor."+str(i) for i in range(1000)]
             +["mutagen."+str(i) for i in range(48)]
             +["websockets."+str(i) for i in range(52)]
             +["curl_cffi."+str(i) for i in range(20)]
             +["unattributed."+str(i) for i in range(454)])
    c=[{"name":"unknown"+str(i)+".pyd","kind":"b"} for i in range(149)]
    c += [{"name":v,"kind":"b"} for v in (
        "THIRD_PARTY_LICENSES.txt","libssl-3.dll",
        "curl_cffi-0.17.0.dist-info\\METADATA",
        "websockets-17.3.dist-info\\METADATA","python314.dll")]
    assert len(c)==154 and len(modules)==1574
    return {"inventory":{"carchive_items":c,"embedded_python_items":[{"name":v} for v in modules]}}

class ReviewTest(unittest.TestCase):
    def test_memory_only_notice_extract_and_bomb_hold(self):
        note=b"THIRD-PARTY LICENSES\nHi"
        self.assertEqual(M.embedded_notice(mock_image(note),[{"name":M.NOTICE_NAME,"kind":"b"}]),note)
        with self.assertRaisesRegex(M.R.Hold,"oversize"):
            M.embedded_notice(mock_image(note,True),[{"name":M.NOTICE_NAME,"kind":"b"}])
        with self.assertRaisesRegex(M.R.Hold,"not found"):
            M.embedded_notice(mock_image(note),[{"name":"other","kind":"b"}])

    def test_upstream_notice_heading_evidence_only(self):
        s=(b"-"*80+b"\nmutagen | GPL-2.0-or-later\n"
           +b"-"*80+b"\nwebsockets | BSD-3-Clause\n")
        self.assertEqual([v["component"] for v in M.notice_sections(s)],["mutagen","websockets"])
        self.assertEqual(M.git_blob_hash(b"hello"),"b6fc4c620b67d95f953a5c1c1230aaab5db5a1b0")

    def test_byte_difference_cannot_become_license_approval(self):
        s=(b"-"*80+b"\nmutagen | GPL-2.0-or-later\n"
           +b"-"*80+b"\nwebsockets | BSD-3-Clause\n")
        o=M.make_report(inventory(),s,s+b"new platform notice text")
        self.assertFalse(o["upstream_notice"]["byte_exact_match"])
        self.assertEqual(o["upstream_notice"]["status"],"DIFF_REQUIRES_MANUAL_REVIEW")
        self.assertFalse(o["upstream_notice"]["newline_normalized_match"])
        self.assertGreater(len(o["upstream_notice"]["changed_normalized_sections"]),0)
        self.assertEqual(o["candidate_group_module_names"],1120)
        self.assertEqual(o["unmapped_module_names"],454)
        f={v["python_root"]:v for v in o["candidate_groups"]}
        self.assertEqual(f["curl_cffi"]["embedded_dist_info_versions"],["0.17.0"])
        self.assertEqual(f["websockets"]["embedded_dist_info_versions"],["17.3"])
        for name in ("release_admitted","legal_approval","license_compatibility_review_complete",
                     "complete_corresponding_source_available","notices_packaging_approved",
                     "binary_constituent_license_review_complete"):
            self.assertFalse(o[name])
        self.assertTrue(all(v["review_status"].startswith("CANDIDATE_ONLY") for v in o["candidate_groups"]))

    def test_crlf_only_difference_is_non_substantive_but_not_legal_approval(self):
        tagged=(b"Header\n"+b"-"*80+b"\nwebsockets | BSD-3-Clause\nNotice text\n"
                +b"-"*80+b"\nmutagen | GPL-2.0-or-later\nOther text\n")
        packed=tagged.replace(b"\n",b"\r\n")
        report=M.make_report(inventory(),tagged,packed)
        review=report["upstream_notice"]
        self.assertFalse(review["byte_exact_match"])
        self.assertTrue(review["newline_normalized_match"])
        self.assertEqual(review["status"],"LINE_ENDING_ONLY")
        self.assertEqual(review["changed_normalized_sections"],[])
        self.assertEqual(review["size_delta_embedded_minus_tagged"],tagged.count(b"\n"))
        self.assertFalse(report["complete_corresponding_source_available"])
        altered=packed.replace(b"Other text",b"Modified notice text")
        report2=M.make_report(inventory(),tagged,altered)
        self.assertEqual(report2["upstream_notice"]["status"],"DIFF_REQUIRES_MANUAL_REVIEW")
        self.assertEqual([x["section"] for x in report2["upstream_notice"]["changed_normalized_sections"]],["mutagen"])

    def test_missing_member_count_is_hold(self):
        obj=inventory()
        obj["inventory"]["carchive_items"].pop()
        with self.assertRaisesRegex(M.R.Hold,"counts changed"):
            M.make_report(obj,b"x",b"y")

    def test_pr_only_workflow(self):
        w=(HERE.parent/"workflows"/"akula-video-tools-01r-embedded-review.yml").read_text("utf8")
        d=(HERE.parent.parent/"docs/release"/"video-tools-01s-notice-reconciliation.md").read_text("utf8")
        for x in (
            ".github/scripts/video-tools-01s-notice-reconciliation.py",
            ".github/scripts/video-tools-01s-notice-reconciliation.test.py",
            "docs/release/video-tools-01s-notice-reconciliation.md",
            "python3 .github/scripts/video-tools-01s-notice-reconciliation.test.py",
            "ref: f264a6cbd1f630894da0f368df00f8aae50140e0",
            "python3 .github/scripts/video-tools-01s-notice-reconciliation.py",
            "video-tools-01s-notice-review-HOLD.json",
            "VIDEO_TOOLS_01S_NOTICE_REVIEW_PASS",
            "retention-days: 1","persist-credentials: false","permissions:\n  contents: read"):
            self.assertIn(x,w)
        for bad in ("workflow_dispatch:","\n  push:","contents: write","gh release create"):
            self.assertNotIn(bad,w)
        for x in ("4,693","GPLv3+","not a complete SBOM","no legal approval","VIDEO-18 RELEASE HOLD"):
            self.assertIn(x,d)

if __name__=="__main__":
    unittest.main(verbosity=2)
