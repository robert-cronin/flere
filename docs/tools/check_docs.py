#!/usr/bin/env python3
"""Check portable handbook links, GitHub anchors, assets, and measured values."""

import gzip, hashlib, json, math, pathlib, re, sys, urllib.parse, xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parents[2]


def prose(path):
    return re.sub(r"^```[^\n]*\n.*?^```\s*$", "", path.read_text(), flags=re.M | re.S)


def anchors(path):
    found = set()
    counts = {}
    for text in re.findall(r"^#{1,6}\s+(.+)$", prose(path), re.M):
        slug = re.sub(r"[^\w\s-]", "", text.lower()).replace(" ", "-")
        count = counts.get(slug, 0)
        counts[slug] = count + 1
        found.add(slug if count == 0 else f"{slug}-{count}")
    return found


errors = []
links = 0
pages = [ROOT / "README.md", *sorted((ROOT / "docs").rglob("*.md"))]
for path in pages:
    content = prose(path)
    targets = re.findall(r'\]\(([^\s)]+)(?:\s+"[^"]*")?\)', content) + re.findall(
        r'(?:src|href)="([^"]+)"', content
    )
    for target in targets:
        url = urllib.parse.urlsplit(target)
        if url.scheme or url.netloc:
            continue
        links += 1
        dest = (
            (path.parent / urllib.parse.unquote(url.path)).resolve()
            if url.path
            else path
        )
        label = f"{path.relative_to(ROOT)} → {target}"
        if not dest.is_relative_to(ROOT):
            errors.append(f"Nonportable path: {label}")
            continue
        if not dest.exists():
            errors.append(f"Missing target: {label}")
            continue
        if (
            url.fragment
            and dest.suffix == ".md"
            and urllib.parse.unquote(url.fragment) not in anchors(dest)
        ):
            errors.append(f"Missing anchor: {label}")
for asset in (ROOT / "docs/assets").iterdir():
    if asset.stat().st_size > 5 * 1024 * 1024:
        errors.append(f"Asset exceeds 5 MiB: {asset.name}")
    if asset.suffix == ".svg":
        tree = ET.parse(asset)
        if any(
            el.tag.split("}")[-1] in ("script", "foreignObject") for el in tree.iter()
        ):
            errors.append(f"Nonportable SVG: {asset.name}")
    if asset.name.endswith(".jsonl.gz"):
        for line in gzip.decompress(asset.read_bytes()).decode().splitlines():
            f = json.loads(line)
            if not (f["cols"] > 0 and f["rows"] > 0 and f["runs"]):
                errors.append(f"Invalid capture: {asset.name}")
for path in (ROOT / "docs/benchmarks").glob("*.json"):
    data = json.loads(path.read_text())
    for case in data["latency"]:
        prefix = "flere" if "flere_completed_frame" in case else "railhand"
        for name in ["direct_pty_echo", f"{prefix}_completed_frame"]:
            values = case[name]["sorted_ms"]
            assert len(values) == case[name]["samples"] == 100
            assert all(math.isfinite(v) and v > 0 for v in values) and values == sorted(
                values
            )
            for key, p in [("p50_ms", 0.5), ("p95_ms", 0.95), ("p99_ms", 0.99)]:
                assert case[name][key] == values[math.ceil(len(values) * p) - 1]
    for name, digest in data["benchmark_source_sha256"].items():
        source_root = ROOT / "docs/benchmarks/railhand-0.2.20" if data["version"].startswith("railhand 0.2.20 ") else ROOT
        if hashlib.sha256((source_root / name).read_bytes()).hexdigest() != digest:
            errors.append(f"Benchmark source differs from recorded run: {name}")
    for case in data["resources"]:
        assert case["seconds"] >= 10 and len(case["samples"]) >= 9
        assert case[f"{prefix}_cpu_percent_one_core"] >= 0
        assert all(sample[f"{prefix}_rss_mib"] > 0 for sample in case["samples"])
if errors:
    print("\n".join(errors))
    sys.exit(1)
print(
    f"Checked {len(pages)} Markdown pages, {links} local links, assets, captures, and benchmark calculations."
)
