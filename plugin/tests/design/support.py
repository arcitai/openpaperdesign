"""Synthetic work roots and an installed read-only package, without source data.

Ported to openpaperdesign from AIOS tests/design/support.py (onlinesourdough/AIOS-plugin
a14f2857, v0.28.1, MIT; see LICENSE): the skill root is plugin/openpaperdesign, and the
OpenPencil adapter is not part of this plugin."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2] / "openpaperdesign"


def fingerprint(root):
    return {str(p.relative_to(root)): (p.stat().st_mode, hashlib.sha256(p.read_bytes()).hexdigest())
            for p in root.rglob("*") if p.is_file() and not p.is_symlink()}


class InstalledCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="portable design ")
        cls.base = Path(cls.temporary.name).resolve()
        cls.package = cls.base / "installed package"
        for name in ("design", "review-design"):
            shutil.copytree(ROOT / "skills" / name, cls.package / "skills" / name)
        cls.cwd = cls.base / "unrelated cwd"
        cls.cwd.mkdir()
        for path in cls.package.rglob("*"):
            path.chmod(0o555 if path.is_dir() else 0o444)
        cls.package.chmod(0o555)
        cls.before = fingerprint(cls.package)

    @classmethod
    def tearDownClass(cls):
        after = fingerprint(cls.package)
        for path in cls.package.rglob("*"):
            path.chmod(0o755 if path.is_dir() else 0o644)
        cls.package.chmod(0o755)
        cls.temporary.cleanup()
        if cls.before != after:
            raise AssertionError("Installed package mutated during execution")

    def invoke(self, skill, script, *args):
        runtime = "python3" if script.endswith(".py") else "node"
        return subprocess.run([runtime, str(self.package / "skills" / skill / "scripts" / script),
                               *map(str, args)], cwd=self.cwd, capture_output=True, text=True,
                              timeout=40, env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"})


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_review(source, reviewer="Independent reviewer", result="PASS", companions=()):
    lines = [f"Reviewer: {reviewer}", f"Result: {result}"]
    for name in ("BRIEF.md", "DESIGN.md"):
        lines.append(f"Reviewed {name} SHA-256: `{digest(source / name)}`")
    for name in companions:
        lines.append(f"Reviewed source companion: `{name}` — SHA-256 `{digest(source / name)}`")
    (source / "REVIEW.md").write_text("\n".join(lines) + "\n")


def fixture(root):
    root.mkdir(parents=True)
    (root / "BRIEF.md").write_text("""# Synthetic direction brief
- **Receiving outcome:** A readable service status interface.
- **Source/reference rights, provenance, and licensing:** Original synthetic test text; no media or private data.
- **Review mode:** independent
- **Review owner:** Independent reviewer
- **Receiver acceptance:** The receiving owner decides separately.
""")
    (root / "DESIGN.md").write_text("""---
name: Service status
version: 1.0.0
---
# Service status
Use readable status labels and a clear primary action.
- **Known limitations:** Synthetic contract proof; no visual acceptance is claimed.
""")
    write_review(root)
    return root
