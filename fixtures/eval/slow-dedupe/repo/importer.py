"""Nightly import: read rows, drop duplicates, hand them on."""

import csv
import json

from dedupe import dedupe


def read_csv(path):
    with open(path, newline="") as f:
        return dedupe(list(csv.DictReader(f)))


def read_jsonl(path):
    with open(path) as f:
        return dedupe([json.loads(line) for line in f if line.strip()])
