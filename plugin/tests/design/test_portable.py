# Cases adapted from ADS test_serve.py and test_handoff.py; MIT, see LICENSE.
# Ported from AIOS tests/design/test_portable.py (a14f2857, MIT); the two OpenPencil cases
# are left out with the adapter, and the proof.json case is kept without the OpenPencil flag.
import json
import re
import subprocess
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

from support import InstalledCase, fixture, fingerprint, write_review


class PortableTests(InstalledCase):
    def handoff(self, source, output, *args, owner="Receiving owner"):
        return self.invoke("design", "create-handoff.mjs", "--design-dir", source,
                           "--output", output, "--receiving-owner", owner, *args)

    def test_minimal_snapshot_and_pending_or_accepted_output_preserved(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = fixture(root / "actual external design")
            before = fingerprint(source)
            output = root / "snapshot"
            result = self.handoff(source, output)
            self.assertEqual(result.returncode, 0, result.stderr)
            data = json.loads(result.stdout)
            self.assertEqual(data["output"], str(output.resolve()))
            self.assertEqual({p.name for p in output.iterdir()}, {"BRIEF.md", "DESIGN.md", "REVIEW.md", "HANDOFF.md"})
            self.assertEqual(data["handoff"]["acceptance"], "PENDING")
            for state in ("PENDING", "ACCEPTED"):
                binder = output / "HANDOFF.md"
                binder.write_text(binder.read_text().replace("Acceptance state: PENDING", f"Acceptance state: {state}"))
                original = fingerprint(output)
                denied = self.handoff(source, output)
                self.assertNotEqual(denied.returncode, 0)
                self.assertEqual(fingerprint(output), original)
            self.assertEqual(fingerprint(source), before)

    def test_review_identity_owner_choice_and_changed_contract(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = fixture(root / "design")
            output = root / "snapshot"
            write_review(source, reviewer="Receiving owner")
            wrong = self.handoff(source, output)
            self.assertEqual(wrong.returncode, 1)
            self.assertIn("does not match", wrong.stderr)
            brief = source / "BRIEF.md"
            brief.write_text(brief.read_text().replace("**Review mode:** independent", "**Review mode:** owner"))
            for reviewer in ("", "independent reviewer", "Receiving owner"):
                write_review(source, reviewer=reviewer)
                waiting = self.handoff(source, output)
                self.assertEqual(waiting.returncode, 2, waiting.stderr)
                self.assertEqual(json.loads(waiting.stderr)["status"], "waiting-owner")
            write_review(source)
            brief.write_text(brief.read_text() + "\nA changed accepted constraint.\n")
            self.assertIn("BRIEF.md SHA-256", self.handoff(source, output).stderr)
            write_review(source)
            self.assertEqual(self.handoff(source, output).returncode, 0)

    def test_stale_direction_unreviewed_and_changed_companions(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = fixture(root / "design")
            output = root / "snapshot"
            design = source / "DESIGN.md"
            design.write_text(design.read_text() + "\nChanged after review.\n")
            self.assertIn("DESIGN.md SHA-256", self.handoff(source, output).stderr)
            write_review(source)
            (source / "assets").mkdir()
            asset = source / "assets/shape.svg"
            asset.write_text("<svg/>\n")
            self.assertIn("does not list selected source companion", self.handoff(source, output, "--asset", "assets/shape.svg").stderr)
            write_review(source, companions=("assets/shape.svg",))
            passed = self.handoff(source, output, "--asset", "assets/shape.svg")
            self.assertEqual(passed.returncode, 0, passed.stderr)
            first_revision = json.loads(passed.stdout)["handoff"]["revision"]
            asset.write_text("<svg>changed</svg>\n")
            self.assertIn("different SHA-256", self.handoff(source, root / "second", "--asset", "assets/shape.svg").stderr)
            write_review(source, companions=("assets/shape.svg",))
            second = self.handoff(source, root / "second", "--asset", "assets/shape.svg")
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertNotEqual(first_revision, json.loads(second.stdout)["handoff"]["revision"])

    def test_selected_exports_need_explicit_external_tool_and_bind_derivation(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = fixture(root / "design")
            output = root / "snapshot"
            missing = self.handoff(source, output, "--export", "tokens")
            self.assertIn("--designmd-cli", missing.stderr)
            self.assertFalse(output.exists())
            tool = root / "fake exporter.mjs"
            tool.write_text('if(process.argv[2]!=="export"||process.argv[4]!=="dtcg")process.exit(3);\nprocess.stdout.write("{\\\"fixture\\\": true}\\n");\n')
            exported = self.handoff(source, output, "--export", "tokens", "--designmd-cli", str(tool))
            self.assertEqual(exported.returncode, 0, exported.stderr)
            data = json.loads(exported.stdout)
            derivation = data["companions"]["deterministicDerivedExports"][0]
            self.assertEqual(derivation["derivedFromDesignSha256"], data["handoff"]["review"]["designSha256"])
            self.assertEqual(json.loads((output / "tokens.json").read_text()), {"fixture": True})

    def test_proof_json_alternative_review_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = fixture(root / "design")
            from support import digest
            (source / "REVIEW.md").unlink()
            (source / "proof.json").write_text(json.dumps({"review": "PASS", "reviewer": "Independent reviewer",
                "reviewed_brief_sha256": digest(source / "BRIEF.md"), "reviewed_design_sha256": digest(source / "DESIGN.md")}))
            result = self.handoff(source, root / "snapshot")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn("openpencil", json.loads(result.stdout))
            self.assertNotIn("OpenPencil", (root / "snapshot" / "HANDOFF.md").read_text())

    def test_selected_preview_is_an_exact_reviewed_copy(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = fixture(root / "design")
            (source / "index.html").write_text("<main>Selected direction</main>\n")
            write_review(source, companions=("index.html",))
            result = self.handoff(source, root / "snapshot", "--preview")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((source / "index.html").read_bytes(), (root / "snapshot" / "index.html").read_bytes())
            audit = self.invoke("review-design", "audit.py", "--snapshot", root / "snapshot")
            self.assertEqual(audit.returncode, 0, audit.stdout)

    def test_escape_symlink_collision_and_package_paths_preserve_data(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            source = fixture(root / "design")
            outside = fixture(root / "outside")
            linked = root / "link"
            linked.symlink_to(outside, target_is_directory=True)
            dangling = root / "dangling"
            dangling.symlink_to(root / "absent")
            before = fingerprint(root)
            for output in (source, root, self.package / "data", linked / "new", dangling):
                self.assertNotEqual(self.handoff(source, output).returncode, 0)
            self.assertNotEqual(self.handoff(source, root / "new", owner="").returncode, 0)
            self.assertNotEqual(self.handoff(linked, root / "new").returncode, 0)
            (source / "REVIEW.md").unlink()
            (source / "REVIEW.md").symlink_to(outside / "REVIEW.md")
            self.assertNotEqual(self.handoff(source, root / "new").returncode, 0)
            self.assertFalse((root / "new").exists())
            self.assertEqual(before["outside/BRIEF.md"], fingerprint(root)["outside/BRIEF.md"])

    def test_preview_binds_loopback_and_denies_escapes_and_bad_urls(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            design = fixture(root / "preview data")
            (design / "index.html").write_text("<main>Selected preview</main>\n")
            outside = root / "outside.txt"
            outside.write_text("must not serve")
            (design / "outside.txt").symlink_to(outside)
            script = self.package / "skills/design/scripts/serve.mjs"
            process = subprocess.Popen(["node", str(script), "--design-dir", str(design), "--port", "0"],
                                       cwd=self.cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                line = process.stdout.readline()
                match = re.search(r"http://127\.0\.0\.1:\d+", line)
                self.assertIsNotNone(match, line)
                origin = match[0]
                with urllib.request.urlopen(origin, timeout=2) as response:
                    self.assertEqual(response.read(), b"<main>Selected preview</main>\n")
                for path, status in (("/outside.txt", 404), ("/..%2Foutside.txt", 404), ("/%E0%A4%A", 400)):
                    with self.assertRaises(urllib.error.HTTPError) as error:
                        urllib.request.urlopen(origin + path, timeout=2)
                    self.assertEqual(error.exception.code, status)
                    error.exception.close()
            finally:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
                process.stdout.close()
                process.stderr.close()
