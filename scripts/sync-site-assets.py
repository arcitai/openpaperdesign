#!/usr/bin/env python3
"""Copy/check pinned repository-owned marketing assets. Never modifies source assets."""
import argparse,hashlib,json
from pathlib import Path
root=Path(__file__).resolve().parents[1];site=root/'site'
parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--check',action='store_true');args=parser.parse_args()
manifest=json.loads((site/'assets-sources.json').read_text())
if manifest['repository']!='arcitai/openpaperdesign':raise ValueError('Wrong asset repository')
for asset in manifest['assets']:
 source=(root/asset['path']).resolve();target=(site/asset['output']).resolve()
 if not source.is_relative_to(root) or not target.is_relative_to(site/'assets'):raise ValueError('Asset path leaves the repository or site/assets')
 data=source.read_bytes()
 if hashlib.sha256(data).hexdigest()!=asset['sha256']:raise ValueError(f"Source hash differs: {asset['path']}")
 if args.check:
  if not target.is_file() or target.read_bytes()!=data:raise ValueError(f"Site asset differs: {target.name}")
 elif source!=target:
  target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(data)
 print(asset['output'])
