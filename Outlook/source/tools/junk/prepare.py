"""Build the bootstrap training pool for Axentra's junk model from the public
SpamAssassin archives, leaving out every message of the 358-message screening
benchmark (and anything that duplicates one).

Writes JSONL (one raw RFC 822 message per line, base64) to the output path.
Mail stays local; nothing is downloaded or uploaded.

Usage: python3 -I tools/junk/prepare.py <archives dir> <benchmark evaluation.jsonl> <out.jsonl>
"""
import base64
import email.policy
import hashlib
import json
import re
import sys
import tarfile
from email.parser import BytesParser
from pathlib import Path

ARCHIVES = [
    ("20030228_easy_ham.tar.bz2", "easy_ham", False),
    ("20030228_hard_ham.tar.bz2", "hard_ham", False),
    ("20050311_spam_2.tar.bz2", "spam_2", True),
]


def norm(s):
    return re.sub(r"\W+", " ", s or "").strip().lower()


def body_key(raw):
    """A campaign key: sender domain-free normalised subject + first 400
    normalised body characters. Near-identical mail shares it."""
    try:
        msg = BytesParser(policy=email.policy.default).parsebytes(raw)
        part = msg.get_body(preferencelist=("plain", "html")) if msg.is_multipart() else msg
        text = part.get_content() if part is not None else ""
    except Exception:
        return None, None
    subject = norm(str(msg.get("Subject", "")))
    return hashlib.sha256((subject + "|" + norm(text)[:400]).encode()).hexdigest()[:16], subject


def main():
    archives, bench_path, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    bench = [json.loads(l) for l in bench_path.open()]
    bench_members = {r["source_member"] for r in bench if r.get("source_member")}
    bench_raw = {r["source_sha256"] for r in bench if r.get("source_sha256")}
    bench_keys = set()
    rows, skipped = [], {"benchmark": 0, "duplicate_of_benchmark": 0, "duplicate": 0, "unreadable": 0}
    pool = []
    for archive, group, unwanted in ARCHIVES:
        with tarfile.open(archives / archive) as tar:
            for m in sorted(tar.getmembers(), key=lambda m: m.name):
                if not m.isfile() or not re.match(r"^\d+\.", Path(m.name).name):
                    continue
                raw = tar.extractfile(m).read()
                key, _ = body_key(raw)
                if m.name in bench_members or hashlib.sha256(raw).hexdigest() in bench_raw:
                    skipped["benchmark"] += 1
                    if key:
                        bench_keys.add(key)
                    continue
                pool.append((m.name, group, unwanted, raw, key))
    seen = set()
    for name, group, unwanted, raw, key in pool:
        if key is None:
            skipped["unreadable"] += 1
            continue
        if key in bench_keys:
            skipped["duplicate_of_benchmark"] += 1
            continue
        if key in seen:
            skipped["duplicate"] += 1
            continue
        seen.add(key)
        rows.append({"id": hashlib.sha256(raw).hexdigest()[:16], "group": key, "source": group, "member": name,
                     "unwanted": unwanted, "raw": base64.b64encode(raw).decode()})
    with out.open("w") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
    counts = {}
    for r in rows:
        counts[r["source"]] = counts.get(r["source"], 0) + 1
    print(json.dumps({"written": len(rows), "by_source": counts, "skipped": skipped}, indent=1))


if __name__ == "__main__":
    main()
