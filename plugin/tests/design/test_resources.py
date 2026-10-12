"""Check all retained skill resources, metadata and portable import targets.

Ported from AIOS tests/design/test_resources.py (a14f2857, MIT). openpaperdesign also checks
its own skills (metadata and links); only the ported skills carry their own LICENSE."""
import re
import unittest

from support import ROOT


class ResourceTests(unittest.TestCase):
    def test_metadata_links_imports_and_no_old_operational_roots(self):
        ported = ("design", "review-design")
        paths = [ROOT / "skills" / name for name in (*ported, "openpaperdesign", "setup")]
        for folder in paths:
            skill = (folder / "SKILL.md").read_text()
            self.assertIn(f"name: {folder.name}\n", skill)
            self.assertRegex(skill, r'metadata:\n  version: "\d+\.\d+\.\d+"')
            self.assertLess(len(skill.encode()), 8192)
            if folder.name in ported:
                self.assertTrue((folder / "LICENSE").is_file())
                self.assertIn("AIOS-plugin", skill, "provenance note")
            for stale in ("openpencil-workbench", "AIOS:"):
                self.assertNotIn(stale, skill)
            for path in folder.rglob("*"):
                if not path.is_file():
                    continue
                text = path.read_text()
                for stale in ("workspace/designs", "workspace/engine", "npm run ", "~/.AIOS", "/Users/", "../../../docs/"):
                    self.assertNotIn(stale, text, str(path))
                if path.suffix == ".md":
                    links = re.findall(r"\[[^\]]*\]\(([^)]+)\)", text)
                    for link in links:
                        if not re.match(r"https?://|#", link):
                            target = (path.parent / link.split("#")[0]).resolve()
                            self.assertTrue(target.is_file(), f"{path}: {link}")
                if path.suffix == ".mjs":
                    for target in re.findall(r'from\s+["\'](\.[^"\']+)["\']', text):
                        self.assertTrue((path.parent / target).resolve().is_file(), f"{path}: {target}")
