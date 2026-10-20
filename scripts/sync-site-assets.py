#!/usr/bin/env python3
"""Refresh the marketing page's pinned, public repository assets."""
import hashlib
import json
from pathlib import Path
from urllib.request import urlopen

site = Path(__file__).resolve().parents[1] / "site"
manifest = json.loads((site / "assets-sources.json").read_text())
if manifest["repository"] != "arcitai/openpaperdesign":
    raise ValueError("Asset sources must belong to arcitai/openpaperdesign")
for asset in manifest["assets"]:
    target = (site / asset["output"]).resolve()
    if not target.is_relative_to((site / "assets").resolve()):
        raise ValueError("Asset output must remain inside site/assets")
    url = f"https://raw.githubusercontent.com/{manifest['repository']}/{asset['revision']}/{asset['path']}"
    with urlopen(url, timeout=20) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != asset["sha256"]:
        raise ValueError(f"Pinned asset hash differs: {asset['path']}")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    print(asset["output"])
