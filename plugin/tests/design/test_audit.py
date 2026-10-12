"""ADS audit outcomes/read-only invariants, adapted to explicit portable scopes."""
import json
import tempfile
from pathlib import Path

from support import InstalledCase, digest, fixture, fingerprint


class AuditTests(InstalledCase):
    def audit(self, *args):
        result = self.invoke("review-design", "audit.py", *args)
        self.assertIn(result.returncode, (0, 1, 2), result.stderr)
        return json.loads(result.stdout)

    def test_pass_fail_blocked_and_all_scopes_read_only(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            healthy = fixture(root / "first design")
            broken = fixture(root / "second design")
            before = fingerprint(root)
            self.assertEqual(self.audit("--design-dir", healthy)["status"], "PASS")
            self.assertEqual(fingerprint(root), before)
            (broken / "REVIEW.md").unlink()
            before = fingerprint(root)
            blocked = self.audit("--design-dir", healthy, "--design-dir", broken)
            self.assertEqual(blocked["status"], "BLOCKED")
            self.assertEqual(len(blocked["evidence"]), 2)
            self.assertEqual(fingerprint(root), before)
            path = healthy / "DESIGN.md"
            path.write_text(path.read_text() + "\nChanged after review.\n")
            before = fingerprint(root)
            failed = self.audit("--design-dir", healthy, "--design-dir", broken)
            self.assertEqual(failed["status"], "FAIL")
            self.assertTrue(any("Stale hash" in gap["message"] for gap in failed["gaps"]))
            self.assertEqual(fingerprint(root), before)

    def test_companion_entries_never_stand_in_for_canonical_evidence(self):
        """Fixed on port (B2b review round 1): a companion named DESIGN.md overrode the
        stale reviewed DESIGN.md hash and the audit returned a false PASS."""
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            # REVIEW.md form: DESIGN.md changed after review, a companion carries the new hash.
            design = fixture(root / "review form")
            old = digest(design / "DESIGN.md")
            path = design / "DESIGN.md"
            path.write_text(path.read_text() + "\nChanged after review.\n")
            review = design / "REVIEW.md"
            review.write_text(review.read_text()
                              + f"Reviewed source companion: `DESIGN.md` — SHA-256 `{digest(path)}`\n")
            out = self.audit("--design-dir", design)
            self.assertEqual(out["status"], "FAIL")
            messages = [gap["message"] for gap in out["gaps"]]
            self.assertTrue(any("Stale hash" in m for m in messages), messages)
            self.assertTrue(any("canonical evidence" in m for m in messages), messages)
            # proof.json form, with every canonical name (and a ./ variant) as a companion.
            proofed = fixture(root / "proof form")
            (proofed / "REVIEW.md").unlink()
            path = proofed / "DESIGN.md"
            path.write_text(path.read_text() + "\nChanged after review.\n")
            proof = {
                "reviewer": "Independent reviewer", "review": "PASS",
                "reviewed_brief_sha256": digest(proofed / "BRIEF.md"),
                "reviewed_design_sha256": old,
                "reviewed_source_companions": {"./DESIGN.md": digest(path)},
            }
            (proofed / "proof.json").write_text(json.dumps(proof))
            out = self.audit("--design-dir", proofed)
            self.assertEqual(out["status"], "FAIL")
            messages = [gap["message"] for gap in out["gaps"]]
            self.assertTrue(any("Stale hash" in m for m in messages), messages)
            for name in ("proof.json", "REVIEW.md", "brief.md"):
                path.write_text(path.read_text())  # unchanged; only the companion key varies
                proof["reviewed_design_sha256"] = digest(path)
                proof["reviewed_source_companions"] = {name: "0" * 64}
                (proofed / "proof.json").write_text(json.dumps(proof))
                out = self.audit("--design-dir", proofed)
                self.assertEqual(out["status"], "FAIL", name)
                self.assertTrue(any("canonical evidence" in g["message"] for g in out["gaps"]), name)
            # Without the collision the same proof passes.
            proof["reviewed_source_companions"] = {}
            (proofed / "proof.json").write_text(json.dumps(proof))
            self.assertEqual(self.audit("--design-dir", proofed)["status"], "PASS")

    def test_snapshot_integrity_acceptance_and_tampering(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = fixture(root / "design")
            output = root / "snapshot"
            handoff = self.invoke("design", "create-handoff.mjs", "--design-dir", source,
                                  "--output", output, "--receiving-owner", "Receiving owner")
            self.assertEqual(handoff.returncode, 0, handoff.stderr)
            before = fingerprint(root)
            result = self.audit("--snapshot", output)
            self.assertEqual(result["status"], "PASS", result)
            self.assertEqual(fingerprint(root), before)
            binder = output / "HANDOFF.md"
            original = binder.read_text()
            binder.write_text(original.replace("Acceptance state: PENDING", "Acceptance state: ACCEPTED"))
            self.assertEqual(self.audit("--snapshot", output)["status"], "FAIL")
            binder.write_text(original.replace("Review mode: independent", "Review mode: owner"))
            self.assertEqual(self.audit("--snapshot", output)["status"], "FAIL")
            binder.write_text(original)
            (output / "unreviewed.txt").write_text("not in manifest")
            before = fingerprint(root)
            self.assertEqual(self.audit("--snapshot", output)["status"], "FAIL")
            self.assertEqual(fingerprint(root), before)
            from support import digest
            binder.write_text(original.replace("## Included snapshot and integrity",
                "## Included snapshot and integrity\n\n- `unreviewed.txt` — SHA-256 `"
                + digest(output / "unreviewed.txt") + "`"))
            result = self.audit("--snapshot", output)
            self.assertEqual(result["status"], "FAIL")
            self.assertTrue(any("no matching review or derivation" in gap["message"] for gap in result["gaps"]))

    def test_missing_or_escaping_scope_and_historical_gap_are_not_promoted(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = fixture(root / "historical design")
            path = source / "REVIEW.md"
            path.write_text("\n".join(line for line in path.read_text().splitlines()
                                      if not line.startswith("Reviewed BRIEF.md")))
            before = fingerprint(root)
            self.assertEqual(self.audit("--design-dir", source)["status"], "BLOCKED")
            self.assertEqual(fingerprint(root), before)
            link = root / "symlink"
            link.symlink_to(source, target_is_directory=True)
            self.assertEqual(self.audit("--design-dir", link)["status"], "FAIL")
            self.assertEqual(self.audit("--design-dir", root / "missing")["status"], "BLOCKED")
