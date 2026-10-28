"""Desktop package publication: exact files, no replacement, safe failure, overlapping runs.

Linux and Windows packages are built from stand-in executables on any host: only the host
check is patched, the archives, notices and file system publication are real. The macOS cases
also run the real sips/iconutil/codesign/ditto and need a Mac (lipo is skipped for stand-ins).
"""
import contextlib
import errno
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
import zipfile
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).parents[1]


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS/filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


packager = load('package_desktop', 'package-desktop.py')
release_manifest = load('release_manifest', 'release-manifest.py')
VERSION = json.loads((SCRIPTS.parent/'packaging/desktop.json').read_text(encoding='utf-8'))['version']
HOSTS = {'linux': 'Linux', 'windows': 'Windows'}
WRITERS = {'linux': tarfile.TarFile, 'windows': zipfile.ZipFile}
MAC_TOOLS = sys.platform == 'darwin' and all(map(shutil.which, ('sips', 'iconutil', 'codesign', 'ditto')))


@contextlib.contextmanager
def after_archive(platform, hook, commands=None):
    """Call hook once a run has completely written its archive, before anything else happens.
    On macOS, commands (a list) receives every command the run executes."""
    if platform == 'macos':
        run = subprocess.run
        def written(command, *args, **kwargs):
            if commands is not None: commands.append(command)
            if command[0] == 'lipo':  # stand-in executables are not the requested slices
                return subprocess.CompletedProcess(command, 0)
            result = run(command, *args, **kwargs)
            if command[0] == 'ditto': hook()
            return result
        with mock.patch.object(packager.subprocess, 'run', written): yield
        return
    close = WRITERS[platform].close
    def written(self):
        close(self)
        if self.mode == 'w' and not getattr(self, 'hooked', False):  # ZipFile.__del__ closes again
            self.hooked = True
            hook()
    with mock.patch.object(WRITERS[platform], 'close', written): yield


class PackagePublication(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.output = self.root/'dist'

    @contextlib.contextmanager
    def host(self, platform):
        # Patched once around all runs: the platform module is shared by every thread.
        with mock.patch.object(packager.host, 'system', return_value=HOSTS[platform]), \
             mock.patch.object(packager.host, 'machine', return_value='x86_64'):
            yield

    def run_package(self, platform, label, arch='x64'):
        """Package the stand-in executables named after label, creating them on first use."""
        executables = self.root/label
        if not executables.exists():
            executables.mkdir()
            for name in ('openpaperdesign', 'paper-rs-mcp'):
                (executables/(name+('.exe' if platform == 'windows' else ''))).write_bytes(f'{label} {name}\n'.encode()*512)
        return packager.package(platform, arch, executables, self.output)

    def mac_executables(self, label):
        """Signable stand-ins: copies of /usr/bin/<label>."""
        (self.root/label).mkdir()
        for name in ('openpaperdesign', 'paper-rs-mcp'):
            shutil.copy(f'/usr/bin/{label}', self.root/label/name)

    def expected(self, platform, arch='x64'):
        return self.output/f'OpenPaperDesign-{VERSION}-{platform}-{arch}{".tar.gz" if platform == "linux" else ".zip"}'

    def snapshot(self):
        return {path.name: (path.read_bytes(), path.stat().st_ino) for path in self.output.iterdir()}

    def staged(self):
        """Every path under the output, relative to it."""
        return sorted(path.relative_to(self.output).as_posix() for path in self.output.rglob('*'))

    def assert_staging(self, platform, label, arch='x64', commands=None):
        """Package label, checking that only its finished archive, then its checksum, are ever under
        the output (in one private directory, gone afterwards). The product is built elsewhere."""
        seen, publish = [], packager.publish
        def published(files, output):
            seen.append(self.staged())
            return publish(files, output)
        with after_archive(platform, lambda: seen.append(self.staged()), commands), \
             mock.patch.object(packager, 'publish', published):
            archive = self.run_package(platform, label, arch)
        private = seen[0][0]
        self.assertRegex(private, rf'^{re.escape(f".OpenPaperDesign-{VERSION}-{platform}-{arch}.")}[^/]+\.partial$')
        self.assertEqual(seen, [[private, f'{private}/{archive.name}'],
                                [private, f'{private}/{archive.name}', f'{private}/{archive.name}.sha256']])
        self.assertEqual(self.staged(), [archive.name, archive.name+'.sha256'])
        self.assert_package(archive, label)
        return archive

    def assert_package(self, archive, label):
        """archive and its checksum are complete, consistent and entirely the work of run label."""
        self.assertTrue(release_manifest.PACKAGE.fullmatch(archive.name), archive.name)
        self.assertEqual(archive.with_name(archive.name+'.sha256').read_text(),
                         f'{release_manifest.digest(archive)}  {archive.name}\n')
        info = release_manifest.build_info(archive)
        self.assertEqual(archive.name, f"OpenPaperDesign-{info['version']}-{info['platform']}-{info['architecture']}"
                         + ('.tar.gz' if info['platform'] == 'linux' else '.zip'))
        self.assertIs(info['automaticUpdates'], False)
        executables = sorted((self.root/label).iterdir())
        self.assertEqual(info['inputBinaries'], {path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in executables})
        if archive.suffix == '.zip':
            with zipfile.ZipFile(archive) as z:
                members = {Path(name).name: z.read(name) for name in z.namelist() if not name.endswith('/')}
        else:
            with tarfile.open(archive, 'r:gz') as t:
                members = {Path(member.name).name: t.extractfile(member).read() for member in t if member.isfile()}
        for notice in ('LICENSE-MIT', 'LICENSE-APACHE', 'THIRD_PARTY.md', 'Inter-OFL.txt'):
            self.assertEqual(members[notice], (SCRIPTS.parent/('assets/fonts/OFL.txt' if notice == 'Inter-OFL.txt' else notice)).read_bytes())
        if info['platform'] != 'macos':  # macOS executables are re-signed inside the bundle
            for path in executables:
                self.assertEqual(members[path.name], path.read_bytes())

    def overlap(self, platform, labels, arch='x64'):
        """Run one package per label at once. Each waits until all have written their archive."""
        barrier = threading.Barrier(len(labels), timeout=120)
        results = {}
        def run(label):
            try:
                results[label] = self.run_package(platform, label, arch)
            except BaseException as error:
                results[label] = error
        workers = [threading.Thread(target=run, args=(label,)) for label in labels]
        with after_archive(platform, barrier.wait):
            for worker in workers: worker.start()
            for worker in workers: worker.join()
        published = {label: result for label, result in results.items() if isinstance(result, Path)}
        refused = [result for result in results.values() if isinstance(result, FileExistsError)]
        self.assertEqual((len(published), len(refused)), (1, len(labels)-1), results)
        (winner, archive), = published.items()
        self.assertEqual(archive, self.expected(platform, arch))
        self.assert_package(archive, winner)
        self.assertEqual(sorted(os.listdir(self.output)), [archive.name, archive.name+'.sha256'])

    def test_distinct_packages_publish_side_by_side(self):
        for platform in ('linux', 'windows'):
            with self.host(platform):
                self.assertEqual(self.run_package(platform, platform), self.expected(platform))
        for platform in ('linux', 'windows'):
            self.assert_package(self.expected(platform), platform)
        self.assertEqual(len(os.listdir(self.output)), 4)

    def test_overlapping_runs_publish_exactly_one_complete_package(self):
        for platform in ('linux', 'windows'):
            self.output = self.root/f'dist-{platform}'
            with self.subTest(platform=platform), self.host(platform):
                self.overlap(platform, [f'{platform}-first', f'{platform}-second', f'{platform}-third'])

    @unittest.skipUnless(MAC_TOOLS, 'needs a Mac with sips, iconutil, codesign and ditto')
    def test_overlapping_macos_runs_publish_exactly_one_complete_package(self):
        for label in ('true', 'false'):
            self.mac_executables(label)
        self.overlap('macos', ['true', 'false'], 'universal')

    def test_only_finished_files_are_staged_beside_the_output(self):
        for platform in ('linux', 'windows'):
            self.output = self.root/f'dist-{platform}'
            with self.subTest(platform=platform), self.host(platform):
                self.assert_staging(platform, platform)

    @unittest.skipUnless(MAC_TOOLS, 'needs a Mac with sips, iconutil, codesign and ditto')
    def test_macos_app_is_signed_in_the_system_temporary_directory_not_the_output(self):
        self.mac_executables('true')
        commands = []
        archive = self.assert_staging('macos', 'true', 'universal', commands)
        system, output = Path(tempfile.gettempdir()).resolve(), self.output.resolve()
        *_, product, built = next(command for command in commands if command[0] == 'ditto')
        product, built = Path(product).resolve(), Path(built).resolve()
        self.assertEqual((product.name, product.parent.parent), ('OpenPaperDesign.app', system))
        self.assertTrue(product.parent.name.startswith('opd-package-'), product)
        self.assertFalse(product.is_relative_to(output), product)
        self.assertFalse(product.parent.exists())
        signed = [Path(command[-1]).resolve() for command in commands if command[0] == 'codesign']
        self.assertEqual(len(signed), 3)  # the bridge, the bundle, then its strict verification
        for path in signed:
            self.assertTrue(path.is_relative_to(product), path)
        self.assertEqual((built.name, built.parent.parent), (archive.name, output))
        with tempfile.TemporaryDirectory() as extracted:
            subprocess.run(['ditto', '-x', '-k', str(archive), extracted], check=True)
            subprocess.run(['codesign', '--verify', '--deep', '--strict', f'{extracted}/OpenPaperDesign.app'], check=True)

    def test_published_package_is_never_replaced(self):
        with self.host('linux'):
            archive = self.run_package('linux', 'first')
            before = self.snapshot()
            with self.assertRaises(FileExistsError):
                self.run_package('linux', 'second')
        self.assertEqual(self.snapshot(), before)
        self.assert_package(archive, 'first')

    def test_unpaired_file_blocks_publication_and_stays_untouched(self):
        archive = self.expected('linux')
        self.output.mkdir()
        with self.host('linux'):
            for existing in (archive, archive.with_name(archive.name+'.sha256')):
                with self.subTest(existing=existing.name):
                    existing.write_bytes(b'kept')
                    before = self.snapshot()
                    with self.assertRaises(FileExistsError):
                        self.run_package('linux', 'run')
                    self.assertEqual(self.snapshot(), before)
                    existing.unlink()

    def test_checksum_appearing_during_the_run_withdraws_only_this_runs_archive(self):
        checksum = self.expected('linux').with_name(self.expected('linux').name+'.sha256')
        with self.host('linux'), after_archive('linux', lambda: checksum.write_bytes(b'other')):
            with self.assertRaises(FileExistsError):
                self.run_package('linux', 'run')
        self.assertEqual(sorted(os.listdir(self.output)), [checksum.name])
        self.assertEqual(checksum.read_bytes(), b'other')

    def test_failed_run_changes_nothing_and_a_retry_publishes(self):
        with self.host('windows'):
            other = self.run_package('windows', 'other')
        before = self.snapshot()
        addfile, calls = tarfile.TarFile.addfile, []
        def full_disk(self, *args, **kwargs):
            calls.append(1)
            if len(calls) > 2: raise OSError(errno.ENOSPC, 'No space left on device')
            return addfile(self, *args, **kwargs)
        with self.host('linux'):
            with mock.patch.object(tarfile.TarFile, 'addfile', full_disk), self.assertRaises(OSError) as failure:
                self.run_package('linux', 'run')
            self.assertEqual(failure.exception.errno, errno.ENOSPC)
            self.assertEqual(self.snapshot(), before)
            archive = self.run_package('linux', 'run')
        self.assert_package(archive, 'run')
        self.assert_package(other, 'other')

    def test_leftovers_of_a_killed_run_neither_block_a_retry_nor_get_removed(self):
        leftover = self.output/f'.OpenPaperDesign-{VERSION}-linux-x64.killed.partial'
        leftover.mkdir(parents=True)
        (leftover/'half').write_bytes(b'half')
        with self.host('linux'):
            archive = self.run_package('linux', 'retry')
        self.assert_package(archive, 'retry')
        self.assertEqual((leftover/'half').read_bytes(), b'half')
        self.assertEqual(sorted(os.listdir(self.output)), sorted([leftover.name, archive.name, archive.name+'.sha256']))


if __name__ == '__main__': unittest.main()
