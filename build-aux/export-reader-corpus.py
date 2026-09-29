#!/usr/bin/env python3
"""Export cached Read/Saved Entry records without accounts, keys, or mutations."""
import argparse
import json
import sqlite3
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('database', type=Path)
parser.add_argument('output', type=Path, help='private output, normally under ignored _build/')
args = parser.parse_args()
connection = sqlite3.connect(args.database.resolve().as_uri() + '?mode=ro', uri=True)
connection.row_factory = sqlite3.Row
with connection:
    rows = connection.execute('''
        SELECT id, account_id, feed_id, feed_title, category_title, title, url,
               author, published_at_ms, html, read, starred, reading_minutes
        FROM entries WHERE read = 1 OR starred = 1
        ORDER BY starred DESC, published_at_ms DESC, id DESC
    ''').fetchall()
entries = []
for row in rows:
    entry = dict(row)
    entry.update(read=bool(entry['read']), starred=bool(entry['starred']),
                 delivery_state=None, delivery_error=None)
    entries.append(entry)
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(entries, ensure_ascii=False), encoding='utf-8')
print(f'Exported {len(entries)} cached articles ({sum(e["starred"] for e in entries)} Saved).')
