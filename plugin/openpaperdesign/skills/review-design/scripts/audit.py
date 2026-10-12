#!/usr/bin/env python3
"""Read-only inspection of explicitly selected design and snapshot evidence.

No directory discovery, dependency, ledger, or mutation. Visual judgment stays
with the review method. Historical proof gaps are reported without upgrading it.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


HASH = r"[0-9a-f]{64}"
PACKAGE = Path(__file__).resolve().parents[3]


class Audit:
    def __init__(self):
        self.evidence = []
        self.gaps = []

    def gap(self, status, scope, message):
        self.gaps.append({"status": status, "scope": str(scope), "message": message})

    def file(self, root, name):
        candidate = root / name
        if Path(name).is_absolute() or ".." in Path(name).parts:
            self.gap("FAIL", root, f"Escaping evidence path: {name}")
            return None
        current = candidate
        while current != root:
            if current.is_symlink():
                self.gap("FAIL", root, f"Symlink evidence path: {name}")
                return None
            current = current.parent
        if not candidate.is_file():
            self.gap("BLOCKED", root, f"Missing regular evidence file: {name}")
            return None
        return candidate

    def read(self, root, name):
        path = self.file(root, name)
        return path.read_text(encoding="utf-8") if path else ""

    def hash_matches(self, root, name, expected):
        if not isinstance(expected, str) or not re.fullmatch(HASH, expected):
            self.gap("BLOCKED", root, f"Missing valid review hash: {name}")
            return
        path = self.file(root, name)
        if path and hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            self.gap("FAIL", root, f"Stale hash: {name}")

    def field(self, text, label, root, strong=False):
        pattern = (rf"^- \*\*{re.escape(label)}:\*\*\s*([^\n]+)" if strong
                   else rf"^{re.escape(label)}:\s*([^\n]+)")
        match = re.search(pattern, text, re.M)
        if not match:
            self.gap("BLOCKED", root, f"Missing field: {label}")
            return ""
        return match[1].strip().strip("`")

    def design(self, root, selected=None):
        brief = self.read(root, "BRIEF.md")
        self.read(root, "DESIGN.md")
        mode = self.field(brief, "Review mode", root, strong=True)
        owner = self.field(brief, "Review owner", root, strong=True)
        if mode and mode not in ("independent", "owner"):
            self.gap("FAIL", root, "Unknown review mode")
        if (root / "REVIEW.md").exists() or (root / "REVIEW.md").is_symlink():
            review = self.read(root, "REVIEW.md")
            reviewer = self.field(review, "Reviewer", root)
            result = self.field(review, "Result", root)
            hashes = {name: self.field(review, f"Reviewed {name} SHA-256", root)
                      for name in ("BRIEF.md", "DESIGN.md")}
            companions = dict(re.findall(
                rf"^Reviewed (?:source )?companion:\s*`([^`]+)`\s+(?:—|-)\s+SHA-256\s+`({HASH})`\s*$",
                review, re.M))
        else:
            raw = self.read(root, "proof.json")
            proof = json.loads(raw) if raw else {}
            reviewer, result = proof.get("reviewer"), proof.get("review")
            hashes = {"BRIEF.md": proof.get("reviewed_brief_sha256"),
                      "DESIGN.md": proof.get("reviewed_design_sha256")}
            companions = proof.get("reviewed_source_companions", proof.get("reviewed_companions", {}))
            if not isinstance(companions, dict):
                self.gap("FAIL", root, "Malformed reviewed companion map")
                companions = {}
        if not reviewer:
            self.gap("BLOCKED", root, "Missing named reviewer")
        elif owner and " ".join(reviewer.split()) != " ".join(owner.split()):
            self.gap("FAIL", root, "Reviewer does not match selected Review owner")
        if result != "PASS":
            self.gap("BLOCKED" if result in (None, "", "BLOCKED") else "FAIL", root,
                     f"No bound PASS: {result or 'missing'}")
        for name, digest in {**hashes, **companions}.items():
            if selected is None or name in hashes or name in selected:
                self.hash_matches(root, name, digest)
        self.evidence.append({"kind": "design", "path": str(root), "review_mode": mode,
                              "review_owner": owner, "result": result})
        return hashes, companions

    def snapshot(self, root):
        binder = self.read(root, "HANDOFF.md")
        integrity = re.search(r"^## Included snapshot and integrity\s*\n(.+?)(?=\n## |\Z)", binder, re.M | re.S)
        entries = re.findall(rf"^- `([^`]+)` — SHA-256 `({HASH})`", integrity[1] if integrity else "", re.M)
        manifest = dict(entries)
        hashes, companions = self.design(root, selected=manifest)
        for label in ("Contract", "Handoff ID", "Handoff revision", "Source", "Source revision",
                      "Receiving owner", "Receiving outcome", "Review state", "Review mode",
                      "Review owner", "Reviewer"):
            value = self.field(binder, label, root)
            if label in ("Review mode", "Review owner"):
                brief_value = self.field(self.read(root, "BRIEF.md"), label, root, strong=True)
                if value and value != brief_value:
                    self.gap("FAIL", root, f"Binder disagrees with brief: {label}")
            if label == "Reviewer" and value and value != self.field(binder, "Review owner", root):
                self.gap("FAIL", root, "Binder reviewer does not match Review owner")
            if label == "Review state" and value and not value.startswith("PASS "):
                self.gap("FAIL", root, "Binder does not record PASS")
            if label == "Contract" and value and value != "ADS-HANDOFF/1":
                self.gap("BLOCKED", root, f"Unsupported snapshot contract: {value}")
        for name, expected in hashes.items():
            recorded = self.field(binder, f"Reviewed {name} SHA-256", root)
            if recorded and expected and recorded != expected:
                self.gap("FAIL", root, f"Binder disagrees with review: {name}")
        for heading in ("Provenance and licensing", "Known limitations"):
            section = re.search(rf"^## {heading}\s*\n(.+?)(?=\n## |\Z)", binder, re.M | re.S)
            if not section or not section[1].strip():
                self.gap("BLOCKED", root, f"Missing {heading}")
        if not entries:
            self.gap("BLOCKED", root, "Missing included-file manifest")
        if len(entries) != len(manifest):
            self.gap("FAIL", root, "Duplicate snapshot manifest path")
        for name, digest in entries:
            self.hash_matches(root, name, digest)
        for name in ("BRIEF.md", "DESIGN.md", "REVIEW.md" if (root / "REVIEW.md").exists() else "proof.json"):
            if name not in manifest:
                self.gap("BLOCKED", root, f"Manifest omits {name}")
        for name, digest in companions.items():
            # A review may cover extra companions that were deliberately not delivered.
            if name in manifest and manifest[name] != digest:
                self.gap("FAIL", root, f"Manifest disagrees with reviewed companion: {name}")
        derived = dict((name, (digest, source_hash)) for name, digest, source_hash in re.findall(
            rf"^- `([^`]+)` — SHA-256 `({HASH})`; derived from reviewed DESIGN.md SHA-256 `({HASH})`",
            binder, re.M))
        core = {"BRIEF.md", "DESIGN.md", "REVIEW.md", "proof.json"}
        for name, digest in manifest.items():
            if name in core or companions.get(name) == digest:
                continue
            if name in ("theme.css", "tokens.json", "tailwind.theme.json") and derived.get(name) == (digest, hashes.get("DESIGN.md")):
                continue
            self.gap("FAIL", root, f"Included companion has no matching review or derivation: {name}")
        for path in root.rglob("*"):
            if path.is_symlink():
                self.gap("FAIL", root, f"Snapshot contains symlink: {path.relative_to(root)}")
            elif path.is_file() and path.name != "HANDOFF.md" and path.relative_to(root).as_posix() not in manifest:
                self.gap("FAIL", root, f"Unbound snapshot file: {path.relative_to(root)}")
        acceptance = self.field(binder, "Acceptance state", root)
        if acceptance and acceptance not in ("PENDING", "ACCEPTED", "REJECTED"):
            self.gap("FAIL", root, "Invalid acceptance state")
        if acceptance == "ACCEPTED":
            for label in ("Accepted by", "Accepted at", "Acceptance statement"):
                value = self.field(binder, label, root)
                if value.startswith("_(") or "must explicitly replace" in value:
                    self.gap("FAIL", root, f"Acceptance remains a placeholder: {label}")
        self.evidence.append({"kind": "snapshot", "path": str(root), "acceptance": acceptance})

    def run(self, designs, snapshots):
        for kind, paths in (("design", designs), ("snapshot", snapshots)):
            for value in paths:
                path = Path(value)
                if not path.is_absolute():
                    self.gap("BLOCKED", value, "Scope requires an absolute path")
                    continue
                try:
                    if any(item.is_symlink() for item in (path, *path.parents)
                           if str(item) not in ("/tmp", "/var")):
                        self.gap("FAIL", value, "Scope traverses a symlink")
                        continue
                    root = path.resolve()
                    if root == PACKAGE or PACKAGE in root.parents or root in PACKAGE.parents:
                        self.gap("BLOCKED", value, "Select work data outside the installed package")
                    elif not root.is_dir():
                        self.gap("BLOCKED", value, "Scope is not an existing directory")
                    else:
                        getattr(self, kind)(root)
                except (OSError, ValueError, TypeError, AttributeError) as error:
                    self.gap("FAIL", value, f"Unreadable or malformed evidence: {error}")
        statuses = {gap["status"] for gap in self.gaps}
        status = "FAIL" if "FAIL" in statuses else "BLOCKED" if self.gaps else "PASS"
        return {"status": status, "evidence": self.evidence, "gaps": self.gaps,
                "next_action": "Keep read-only; no correction needed in this scope." if status == "PASS"
                else "Give the listed gaps to the existing writer/reviewer; preserve current evidence."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--design-dir", action="append", default=[])
    parser.add_argument("--snapshot", action="append", default=[])
    args = parser.parse_args()
    if not args.design_dir and not args.snapshot:
        parser.error("Select at least one --design-dir or --snapshot.")
    result = Audit().run(args.design_dir, args.snapshot)
    print(json.dumps(result, indent=2))
    return {"PASS": 0, "FAIL": 1, "BLOCKED": 2}[result["status"]]


if __name__ == "__main__":
    raise SystemExit(main())
