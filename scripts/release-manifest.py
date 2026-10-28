#!/usr/bin/env python3
"""Verify all three desktop packages before writing a release download manifest."""
import argparse
import hashlib
import json
import re
import tarfile
import zipfile
from pathlib import Path

PACKAGE = re.compile(r'OpenPaperDesign-([0-9][A-Za-z0-9.-]*)-(macos-universal|windows-x64|linux-x64)\.(zip|tar\.gz)\Z')


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def build_info(path):
    if path.suffix == '.zip':
        with zipfile.ZipFile(path) as archive:
            entries = [entry for entry in archive.infolist() if entry.filename.endswith('/BUILD.json')]
            if len(entries) != 1 or entries[0].file_size > 65536:
                raise ValueError('Expected exactly one bounded BUILD.json')
            return json.loads(archive.read(entries[0]))
    with tarfile.open(path, 'r:gz') as archive:
        entries = [entry for entry in archive if entry.name.endswith('/BUILD.json')]
        if len(entries) != 1 or not entries[0].isfile() or entries[0].size > 65536:
            raise ValueError('Expected exactly one bounded BUILD.json')
        return json.load(archive.extractfile(entries[0]))


def manifest(directory, repository, tag, revision):
    if repository != 'arcitai/openpaperdesign' or not re.fullmatch(r'v[0-9][A-Za-z0-9.-]*', tag):
        raise ValueError('Unexpected release destination or tag')
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise ValueError('Expected an exact source revision')
    platforms = {}
    version = None
    for path in sorted(directory.iterdir()):
        match = PACKAGE.fullmatch(path.name)
        if not match:
            continue
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError('Invalid package file')
        package_version, target, extension = match.groups()
        platform, arch = target.split('-')
        if extension != ('tar.gz' if platform == 'linux' else 'zip') or platform in platforms:
            raise ValueError('Unexpected or duplicate platform package')
        if version is not None and package_version != version:
            raise ValueError('Mixed package versions')
        version = package_version
        checksum = digest(path)
        if path.with_name(path.name+'.sha256').read_text().strip() != f'{checksum}  {path.name}':
            raise ValueError(f'Checksum mismatch: {path.name}')
        info = build_info(path)
        if (info.get('dirty') is not False or info.get('revision') != revision
                or info.get('platform') != platform or info.get('architecture') != arch
                or info.get('version') != version):
            raise ValueError(f'Unqualified package provenance: {path.name}')
        platforms[platform] = {
            'url': f'https://github.com/{repository}/releases/download/{tag}/{path.name}',
            'architecture': 'Intel + Apple Silicon' if arch == 'universal' else arch,
            'status': 'Desktop preview · ad-hoc signed, not notarized' if platform == 'macos' else 'Desktop preview',
            'sha256': checksum, 'bytes': path.stat().st_size,
        }
    if set(platforms) != {'macos', 'windows', 'linux'}:
        raise ValueError('All three verified platform packages are required')
    if not (tag == 'v'+version or tag.startswith('v'+version+'-')):
        raise ValueError('Tag does not match package version')
    return {'version': version, 'revision': revision, 'tag': tag, 'platforms': platforms,
            'notice': 'Desktop preview. Check platform notes, packaged licences and checksums before use.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--revision', required=True)
    args = parser.parse_args()
    value = manifest(args.directory, args.repository, args.tag, args.revision)
    (args.directory/'releases.json').write_text(json.dumps(value, indent=2)+'\n')
