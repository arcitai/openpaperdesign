#!/usr/bin/env python3
"""Export the committed static site; ignored local previews never enter a deployment."""
import argparse
import io
from pathlib import Path, PurePosixPath
import subprocess
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', required=True, type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
if args.output.exists():
    parser.error('Choose a new output directory; existing files are never overwritten.')
archive = subprocess.check_output(['git', 'archive', '--format=tar', 'HEAD', 'site'], cwd=root)
with tarfile.open(fileobj=io.BytesIO(archive)) as source:
    files = []
    for member in source.getmembers():
        path = PurePosixPath(member.name)
        if member.isdir():
            continue
        if not member.isfile() or path.parts[0] != 'site' or '..' in path.parts:
            raise ValueError('Unexpected site archive entry')
        relative = Path(*path.parts[1:])
        if relative.name == 'releases.preview.json' or relative.parts[0] == 'downloads':
            raise ValueError('A local preview was committed; refuse deployment export')
        files.append((member, relative))
    args.output.mkdir(parents=True)
    for member, relative in files:
        target = args.output / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.extractfile(member).read())
print(f'Exported {len(files)} committed files to {args.output}')
