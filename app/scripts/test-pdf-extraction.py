#!/usr/bin/env python3
"""Verify the real extraction worker includes persisted annotations in image and text.

Usage: test-pdf-extraction.py /path/to/stology-extract /path/to/gate-fixtures
Create fixtures first with test-pdf-annotations.swift. Uses isolated temporary caches.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

worker, fixtures = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
marker = "세포 에너지 한글 메모 받침 수정 ABC 123"
with tempfile.TemporaryDirectory(prefix="stology-pdf-extraction-") as cache:
    results = []
    for name in ("source", "annotated"):
        response = subprocess.run(
            [str(worker), str(fixtures / f"{name}.pdf"), str(Path(cache) / name)],
            check=True, text=True, capture_output=True,
        )
        results.append(json.loads(response.stdout))
    before, after = (result["units"] for result in results)
    assert len(before) == len(after) == 4, "fixture has four rotated pages"
    for index, (old, new) in enumerate(zip(before, after)):
        assert marker not in old["text"], "source unexpectedly contains memo"
        assert marker in new["text"], f"memo omitted on page {index + 1}"
        assert "Body content remains" in new["text"], "body text lost"
        old_image = Path(old["images"][0]["path"]).read_bytes()
        new_image = Path(new["images"][0]["path"]).read_bytes()
        assert hashlib.sha256(old_image).digest() != hashlib.sha256(new_image).digest(), (
            f"annotation image omitted on page {index + 1}"
        )
    print(json.dumps({"extraction": "PASS", "rotations": [0, 90, 180, 270],
                      "imagesIncludeAnnotations": True, "memoTextIncluded": True,
                      "bodyTextPreserved": True}, ensure_ascii=False))
