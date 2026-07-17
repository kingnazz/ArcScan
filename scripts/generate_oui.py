#!/usr/bin/env python3
"""Compress the IEEE MA-L OUI registry into a compact embedded data file.

The real registry (https://standards-oui.ieee.org/oui/oui.csv, ~39k rows) is
far too large and messy to embed as-is. This script parses it down to a
newline-delimited `PREFIX\tVendor` table (PREFIX = 6 uppercase hex chars) that
the Rust backend embeds via include_str! and loads into a HashMap once.

Usage:
  python3 scripts/generate_oui.py [path/to/oui.csv]

If no path is given it downloads the CSV. The output is written to
src-tauri/src/oui_data.tsv and should be committed to the repo so builds are
reproducible offline.
"""
import csv
import io
import os
import sys

OUT = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "src", "oui_data.tsv")
URL = "https://standards-oui.ieee.org/oui/oui.csv"


def load_csv_text(argv):
    if len(argv) > 1:
        with open(argv[1], "r", encoding="utf-8", errors="replace") as f:
            return f.read()
    import urllib.request
    print(f"downloading {URL} ...")
    with urllib.request.urlopen(URL, timeout=120) as resp:
        return resp.read().decode("utf-8", errors="replace")


def clean_vendor(name):
    name = name.strip().strip('"').strip()
    # collapse whitespace
    name = " ".join(name.split())
    # trim overly long trailing corporate cruft but keep it readable
    if len(name) > 60:
        name = name[:60].rstrip()
    return name


def main():
    text = load_csv_text(sys.argv)
    reader = csv.reader(io.StringIO(text))
    header = next(reader, None)
    # Expected columns: Registry,Assignment,Organization Name,Organization Address
    rows = {}
    for row in reader:
        if len(row) < 3:
            continue
        prefix = row[1].strip().upper().replace("-", "").replace(":", "")
        if len(prefix) != 6 or not all(c in "0123456789ABCDEF" for c in prefix):
            continue
        vendor = clean_vendor(row[2])
        if not vendor:
            continue
        rows[prefix] = vendor

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for prefix in sorted(rows):
            f.write(f"{prefix}\t{rows[prefix]}\n")

    size = os.path.getsize(OUT)
    print(f"wrote {len(rows)} vendor prefixes -> {OUT} ({size/1024:.0f} KiB)")


if __name__ == "__main__":
    main()
