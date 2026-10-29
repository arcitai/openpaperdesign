import hashlib
import importlib.util
import io
import json
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

spec = importlib.util.spec_from_file_location('release_manifest', Path(__file__).parents[1]/'release-manifest.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
REV = 'a'*40


class ReleaseManifest(unittest.TestCase):
    def packages(self, root):
        for platform, arch in [('macos','universal'), ('windows','x64'), ('linux','x64')]:
            info = json.dumps({'dirty': False, 'revision': REV, 'platform': platform,
                               'architecture': arch, 'version': '0.1.0'}).encode()
            name = f'OpenPaperDesign-0.1.0-{platform}-{arch}'
            path = root/(name+('.tar.gz' if platform == 'linux' else '.zip'))
            if platform == 'linux':
                with tarfile.open(path, 'w:gz') as archive:
                    member = tarfile.TarInfo(name+'/BUILD.json'); member.size=len(info)
                    archive.addfile(member, io.BytesIO(info))
            else:
                with zipfile.ZipFile(path, 'w') as archive: archive.writestr(name+'/BUILD.json',info)
            path.with_name(path.name+'.sha256').write_text(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n')
        return root

    def test_complete_exact_archives_and_damaged_or_incomplete_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=self.packages(Path(temporary))
            value=module.manifest(root,'arcitai/openpaperdesign','v0.1.0-desktop.1',REV)
            self.assertEqual(set(value['platforms']),{'macos','windows','linux'})
            self.assertEqual(value['platforms']['macos']['architecture'],'Intel + Apple Silicon')
            with self.assertRaises(ValueError): module.manifest(root,'arcitai/openpaperdesign','v0.1.0-desktop.1','b'*40)
            path=root/'OpenPaperDesign-0.1.0-windows-x64.zip'
            path.write_bytes(b'damaged')
            with self.assertRaises(ValueError): module.manifest(root,'arcitai/openpaperdesign','v0.1.0-desktop.1',REV)
            path.unlink()
            with self.assertRaises(ValueError): module.manifest(root,'arcitai/openpaperdesign','v0.1.0-desktop.1',REV)

if __name__ == '__main__': unittest.main()
