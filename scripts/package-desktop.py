#!/usr/bin/env python3
"""Package existing, target-built native binaries. No downloads, signing credentials or publish.

Usage: package-desktop.py --platform macos|windows|linux --arch arm64|x64
                         --bin-dir PATH --output PATH
macOS packaging runs on a Mac and uses ad-hoc signing only. Release signing/notarization
is deliberately separate; a successful archive does not prove end-user installation.
"""
import argparse
import hashlib
import json
import plistlib
import platform as host
import shutil
import subprocess
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def package(platform, arch, binaries, output):
    expected = {"macos": "Darwin", "windows": "Windows", "linux": "Linux"}[platform]
    if host.system() != expected:
        raise ValueError(f"Package {platform} on its matching build host, not {host.system()}")
    machine = host.machine().lower()
    actual_arch = "arm64" if machine in ("arm64", "aarch64") else "x64" if machine in ("x86_64", "amd64") else machine
    if actual_arch != arch:
        raise ValueError(f"Requested {arch} but the build host is {actual_arch}")
    config = json.loads((ROOT / "packaging/desktop.json").read_text())
    suffix = ".exe" if platform == "windows" else ""
    source = binaries / ("openpaperdesign" + suffix)
    bridge = binaries / ("paper-rs-mcp" + suffix)
    for required in (source, bridge):
        if not required.is_file():
            raise FileNotFoundError(f"Missing built executable: {required}")
    output.mkdir(parents=True, exist_ok=True)
    name = f"OpenPaperDesign-{config['version']}-{platform}-{arch}"
    with tempfile.TemporaryDirectory(prefix="opd-package-") as temporary:
        stage = Path(temporary)
        if platform == "macos":
            product = stage / "OpenPaperDesign.app"
            executable_dir = product / "Contents/MacOS"
            resources = product / "Contents/Resources"
            executable_dir.mkdir(parents=True)
            resources.mkdir(parents=True)
            plist = {"CFBundleName": config["name"], "CFBundleDisplayName": config["name"],
                     "CFBundleIdentifier": config["identifier"], "CFBundleExecutable": "openpaperdesign",
                     "CFBundlePackageType": "APPL", "CFBundleShortVersionString": config["version"],
                     "CFBundleVersion": config["version"], "LSMinimumSystemVersion": config["minimumMacOS"],
                     "NSHighResolutionCapable": True, "LSApplicationCategoryType": "public.app-category.graphics-design",
                     "NSHumanReadableCopyright": "OpenPaperDesign / Arc’IT AI. Built on paper-rs by Michael Shimeles.",
                     "CFBundleIconFile": "AppIcon"}
            (product / "Contents/Info.plist").write_bytes(plistlib.dumps(plist))
            iconset = stage / "AppIcon.iconset"
            iconset.mkdir()
            icon = ROOT / "packaging/icons/icon-1024.png"
            if not icon.is_file():
                raise FileNotFoundError("Render packaging/icons/icon-1024.png before packaging")
            for size in (16, 32, 128, 256, 512):
                for factor in (1, 2):
                    target = iconset / f"icon_{size}x{size}{'@2x' if factor == 2 else ''}.png"
                    subprocess.run(["sips", "-z", str(size * factor), str(size * factor), str(icon), "--out", str(target)], check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(resources / "AppIcon.icns")], check=True)
        else:
            product = stage / name
            executable_dir = resources = product
            product.mkdir()
        for path in (source, bridge):
            destination = executable_dir / path.name
            shutil.copy2(path, destination)
            destination.chmod(0o755)
        for license_file in ("LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY.md"):
            shutil.copy2(ROOT / license_file, resources / license_file)
        shutil.copy2(ROOT / "assets/fonts/OFL.txt", resources / "Inter-OFL.txt")
        (resources / "BUILD.json").write_text(json.dumps({**config, "platform": platform, "architecture": arch,
            "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)),
            "automaticUpdates": False}, indent=2) + "\n")
        if platform == "macos":
            subprocess.run(["codesign", "--force", "--sign", "-", str(executable_dir / bridge.name)], check=True)
            subprocess.run(["codesign", "--force", "--sign", "-", str(product)], check=True)
            subprocess.run(["codesign", "--verify", "--deep", "--strict", str(product)], check=True)
            archive = output / (name + ".zip")
            subprocess.run(["ditto", "-c", "-k", "--keepParent", str(product), str(archive)], check=True)
        elif platform == "windows":
            archive = output / (name + ".zip")
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as z:
                for path in sorted(product.rglob("*")):
                    if path.is_file():
                        z.write(path, path.relative_to(stage))
        else:
            archive = output / (name + ".tar.gz")
            with tarfile.open(archive, "w:gz") as t:
                t.add(product, arcname=product.name)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n")
        print(archive)
        return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("macos", "windows", "linux"), required=True)
    parser.add_argument("--arch", choices=("arm64", "x64"), required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    package(args.platform, args.arch, args.bin_dir.resolve(), args.output.resolve())
