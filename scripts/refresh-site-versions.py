#!/usr/bin/env python3
"""Keep plain-static preview HTML/modules bound to their actual stylesheet/JS bytes."""
import argparse
import hashlib
import re
from pathlib import Path

site = Path(__file__).resolve().parents[1] / 'site'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--check', action='store_true')
args = parser.parse_args()

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()[:12]

module = site / 'site.js'
before = module.read_text()
after = re.sub(r"from './downloads\.js(?:\?v=[0-9a-f]+)?'", f"from './downloads.js?v={digest(site / 'downloads.js')}'", before)
if args.check:
    if before != after:
        raise SystemExit('Refresh the download module version')
else:
    module.write_text(after)

index = site / 'index.html'
before = index.read_text()
after = before
for name in ('styles.css', 'site.js'):
    after = re.sub(r'(["\'])/' + re.escape(name) + r'(?:\?v=[0-9a-f]+)?(["\'])', lambda m: f'{m[1]}/{name}?v={digest(site / name)}{m[2]}', after)
if args.check:
    if before != after:
        raise SystemExit('Refresh the stylesheet and script versions')
else:
    index.write_text(after)
print('Site asset versions match content')
