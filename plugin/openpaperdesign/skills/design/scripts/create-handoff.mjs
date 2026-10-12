// Adapted from ADS 52322ed; MIT, see ../LICENSE.
// openpaperdesign: ported from AIOS skills/design/scripts/create-handoff.mjs at
// onlinesourdough/AIOS-plugin a14f2857 (v0.28.1, MIT); the optional OpenPencil route was
// removed (see ../../../NOTICE.md).
import {
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from "node:path";
import { spawnSync } from "node:child_process";
import { selectDesign, externalPath, sourceFile, pathExists } from "./paths.mjs";

const { positional, options } = parseArguments(process.argv.slice(2));
if (positional.length)
  fail(
    "Handoff uses --design-dir <absolute-directory> and --output <absolute-absent-directory>.",
  );
const receivingOwner = options.receivingOwner?.trim();
if (!receivingOwner)
  fail(
    "Pass a non-empty --receiving-owner for the receiving project or repository.",
  );
if (!options.designDir) fail("Pass --design-dir <absolute-work-directory>.");
if (!options.output) fail("Pass --output <absolute-absent-snapshot-directory>.");
let source, output;
try {
  source = selectDesign(options.designDir);
  output = externalPath(options.output);
  assertSafeOutput(output, source);
  for (const file of ["BRIEF.md", "DESIGN.md"])
    sourceFile(source, file);
  for (const file of ["REVIEW.md", "proof.json"])
    if (pathExists(join(source, file))) sourceFile(source, file);
} catch (error) { fail(error.message); }
const designmd = options.designmdCli ? resolve(options.designmdCli) : null;

const portable = inspectPortableContract(source, receivingOwner);
const companions = inspectCompanions(source, options, designmd);
const reviewedSourceCompanions = assertReviewedSourceCompanions(
  portable.review,
  companions,
);
const reviewBoundary = {
  reviewedSourceCompanions,
  deterministicDerivedExports: companions.exports.map((artifact) => ({
    path: artifact.handoffPath,
    sha256: artifact.sha256,
    derivedFromDesignSha256: portable.review.reviewedDesignSha256,
  })),
};

// Capture final selected bytes before creating a new snapshot. Never replace even
// a pending output: it may contain owner edits or recovery evidence.
const copies = ["BRIEF.md", "DESIGN.md", portable.review.evidence].map((name) => ({
  handoffPath: name, absolutePath: sourceFile(source, name), sha256: sha256(sourceFile(source, name)),
}));
if (copies[0].sha256 !== portable.review.reviewedBriefSha256 ||
    copies[1].sha256 !== portable.review.reviewedDesignSha256 ||
    copies[2].sha256 !== portable.review.evidenceSha256)
  fail("Reviewed source changed during generation.");
copies.push(...companions.files);
const payload = new Map();
for (const item of copies) {
  const bytes = readFileSync(item.absolutePath);
  if (createHash("sha256").update(bytes).digest("hex") !== item.sha256)
    fail(`Selected source changed: ${item.handoffPath}`);
  if (payload.has(item.handoffPath)) fail(`Duplicate snapshot path: ${item.handoffPath}`);
  payload.set(item.handoffPath, bytes);
}
for (const item of companions.exports) payload.set(item.handoffPath, Buffer.from(item.content));
const manifest = [...payload].map(([path, bytes]) => ({
  path, sha256: createHash("sha256").update(bytes).digest("hex"),
})).sort((a, b) => a.path.localeCompare(b.path));
portable.revision = `${portable.version}+${createHash("sha256").update(JSON.stringify(manifest)).digest("hex").slice(0, 12)}`;
payload.set("HANDOFF.md", Buffer.from(handoffMarkdown({
  manifest, companions, portable, receivingOwner, reviewBoundary, source,
})));
if (pathExists(output)) fail("Handoff output already exists; preserve the snapshot and choose a new revision path.");
// Create only the explicitly selected leaf; its parent must already exist.
try { mkdirSync(output); } catch (error) { fail(error.message); }
try {
  for (const [name, bytes] of payload) {
    const target = join(output, name);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, bytes, { flag: "wx" });
  }
} catch (error) {
  // Retain partial output for inspection. A retry selects a new directory.
  fail(`Incomplete snapshot retained at ${output}: ${error.message}`);
}

process.stdout.write(
  `${JSON.stringify(
    {
      success: true,
      source,
      output,
      design: basename(join(source, "DESIGN.md")),
      handoff: {
        contract: portable.contract,
        id: portable.id,
        revision: portable.revision,
        sourceRevision: portable.sourceRevision,
        receivingOutcome: portable.receivingOutcome,
        review: publicReview(portable.review, reviewBoundary),
        acceptance: "PENDING",
        artifacts: manifest,
      },
      receivingOwner,
      companions: {
        reviewedSourceCompanions: reviewBoundary.reviewedSourceCompanions,
        deterministicDerivedExports: reviewBoundary.deterministicDerivedExports,
      },
    },
    null,
    2,
  )}\n`,
);

function parseArguments(args) {
  const positional = [];
  const options = { assets: [], exports: [] };
  const values = new Map([
    ["--design-dir", "designDir"],
    ["--designmd-cli", "designmdCli"],
    ["--output", "output"],
    ["--receiving-owner", "receivingOwner"],
    ["--asset", "assets"],
    ["--export", "exports"],
  ]);
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === "--preview") {
      options.preview = true;
      continue;
    }
    if (values.has(argument)) {
      const value = args[index + 1];
      if (!value || value.startsWith("--"))
        fail(`${argument} requires a value.`);
      const key = values.get(argument);
      if (["assets", "exports"].includes(key))
        options[key].push(value);
      else options[key] = value;
      index += 1;
      continue;
    }
    if (argument.startsWith("--")) fail(`Unknown handoff option: ${argument}`);
    positional.push(argument);
  }
  return { positional, options };
}

function inspectCompanions(sourceRoot, values, exporter) {
  const files = [];
  let preview = false;
  if (values.preview) {
    try {
      files.push(selectedFile(sourceRoot, "index.html", "Preview"));
      preview = true;
    } catch (error) {
      fail(error.message);
    }
  }

  const assets = [];
  for (const selected of values.assets) {
    let item;
    try {
      item = selectedFile(sourceRoot, selected, "Selected asset");
    } catch (error) {
      fail(error.message);
    }
    if (!item.handoffPath.startsWith("assets/"))
      fail(`--asset paths must be under assets/: ${selected}`);
    assets.push(item);
    files.push(item);
  }

  const formats = new Map([
    ["css", { format: "css-tailwind", handoffPath: "theme.css" }],
    ["tokens", { format: "dtcg", handoffPath: "tokens.json" }],
    [
      "tailwind",
      { format: "json-tailwind", handoffPath: "tailwind.theme.json" },
    ],
  ]);
  const selectedExports = [...new Set(values.exports)];
  for (const selected of selectedExports) {
    if (!formats.has(selected))
      fail(
        `--export must be one of css, tokens, or tailwind; received ${selected}.`,
      );
  }
  if (selectedExports.length && (!exporter || !existsSync(exporter)))
    fail(
      "Selected token exports require an external @google/design.md@0.3.0 CLI via --designmd-cli; no tool is installed by this helper.",
    );
  const exports = selectedExports.map((selected) => {
    const { format, handoffPath } = formats.get(selected);
    const exported = spawnSync(
      process.execPath,
      [exporter, "export", "--format", format, join(sourceRoot, "DESIGN.md")],
      { encoding: "utf8", cwd: sourceRoot, timeout: 30000 },
    );
    if (exported.status !== 0 || !exported.stdout.trim())
      fail(exported.stderr.trim() || `Export failed for ${format}.`);
    return {
      handoffPath,
      content: `${exported.stdout.trimEnd()}\n`,
      sha256: createHash("sha256")
        .update(`${exported.stdout.trimEnd()}\n`)
        .digest("hex"),
    };
  });

  return { preview, assets, exports, files };
}

function selectedFile(sourceRoot, sourceRelativePath, label) {
  const absolutePath = sourceFile(sourceRoot, sourceRelativePath);
  return { absolutePath, handoffPath: relative(sourceRoot, absolutePath).split(sep).join("/"), sha256: sha256(absolutePath) };
}

function inspectPortableContract(sourceRoot, owner) {
  const briefPath = join(sourceRoot, "BRIEF.md");
  const designPath = join(sourceRoot, "DESIGN.md");
  const brief = readFileSync(briefPath, "utf8");
  const design = readFileSync(designPath, "utf8");
  const receivingOutcome = requiredStrongField(
    brief,
    "Receiving outcome",
    "BRIEF.md",
  );
  const provenance = requiredStrongField(
    brief,
    "Source/reference rights, provenance, and licensing",
    "BRIEF.md",
  );
  const limitations = requiredStrongField(
    design,
    "Known limitations",
    "DESIGN.md",
  );
  const reviewMode = requiredStrongField(
    brief,
    "Review mode",
    "BRIEF.md",
  ).toLowerCase();
  if (!["independent", "owner"].includes(reviewMode))
    fail(
      `BRIEF.md **Review mode:** must be exactly independent or owner; received ${reviewMode}.`,
    );
  const reviewOwner = requiredStrongField(brief, "Review owner", "BRIEF.md");
  const frontmatter = design.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/)?.[1] ?? "";
  const version = frontmatter.match(/^version:\s*["']?([^\n"']+)["']?\s*$/m)?.[1];
  const name = frontmatter.match(/^name:\s*["']?([^\n"']+)["']?\s*$/m)?.[1];
  if (!version || !name)
    fail(
      "DESIGN.md must declare non-empty frontmatter version and name values.",
    );

  const designSha256 = sha256(designPath);
  const briefSha256 = sha256(briefPath);
  const review = {
    ...inspectReview(sourceRoot),
    mode: reviewMode,
    reviewOwner,
  };
  const reviewProblem = reviewGateProblem(review, designSha256, briefSha256);
  const identityProblem =
    review.reviewer &&
    normalizeIdentity(review.reviewer) !== normalizeIdentity(reviewOwner)
      ? `${review.evidence} Reviewer ${review.reviewer} does not match BRIEF.md Review owner ${reviewOwner}.`
      : null;
  if (reviewMode === "owner") {
    if (reviewProblem || identityProblem)
      waitingOwner(reviewProblem ?? identityProblem);
  } else {
    if (reviewProblem || identityProblem)
      fail(reviewProblem ?? identityProblem);
  }
  const identitySeed = `${name.trim()}\n${owner}\n${receivingOutcome}`;
  const identity = createHash("sha256")
    .update(identitySeed)
    .digest("hex")
    .slice(0, 12);
  const slug = name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 40);
  return {
    contract: "ADS-HANDOFF/1",
    version: version.trim(),
    id: `ads-${slug || "design"}-${identity}`,
    revision: `${version.trim()}+${designSha256.slice(0, 12)}`,
    sourceRevision: `DESIGN.md ${version.trim()} · SHA-256 ${designSha256}`,
    receivingOutcome,
    provenance,
    limitations,
    review,
  };
}

function requiredStrongField(markdown, label, filename) {
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = markdown.match(
    new RegExp(
      `^- \\*\\*${escaped}:\\*\\*\\s+([\\s\\S]*?)(?=\\n- \\*\\*|\\n#{1,6} |(?![\\s\\S]))`,
      "m",
    ),
  );
  const value = match?.[1]?.replace(/\s+/g, " ").trim();
  if (!value) fail(`${filename} lacks a non-empty **${label}:** field.`);
  return value;
}

function inspectReview(sourceRoot) {
  const reviewPath = join(sourceRoot, "REVIEW.md");
  if (existsSync(reviewPath)) {
    const review = readFileSync(reviewPath, "utf8");
    // Contradictory evidence is refused, never resolved by keeping one of the records.
    for (const label of ["Result", "Reviewer", "Reviewed DESIGN\\.md SHA-256", "Reviewed BRIEF\\.md SHA-256"]) {
      if ((review.match(new RegExp(`^${label}:`, "gm")) ?? []).length > 1)
        return { error: `REVIEW.md repeats ${label.replaceAll("\\", "")}.` };
    }
    const status = review.match(/^Result:\s*(PASS|REVISE|BLOCKED)\s*$/m)?.[1];
    const reviewer = review.match(/^Reviewer:\s*(\S[^\n]*)\s*$/m)?.[1]?.trim();
    const reviewedDesignSha256 = review.match(
      /^Reviewed DESIGN\.md SHA-256:\s*`?([0-9a-f]{64})`?\s*$/m,
    )?.[1];
    const reviewedBriefSha256 = review.match(/^Reviewed BRIEF\.md SHA-256:\s*`?([0-9a-f]{64})`?\s*$/m)?.[1];
    const reviewedSourceCompanions = {};
    for (const match of review.matchAll(
      /^Reviewed (?:source )?companion:\s*`([^`]+)`\s+(?:—|-)\s+SHA-256\s+`([0-9a-f]{64})`\s*$/gm,
    )) {
      if (Object.hasOwn(reviewedSourceCompanions, match[1]))
        return { error: `REVIEW.md lists reviewed companion ${match[1]} more than once.` };
      reviewedSourceCompanions[match[1]] = match[2];
    }
    return {
      status,
      reviewer,
      reviewedDesignSha256,
      reviewedBriefSha256,
      reviewedSourceCompanions,
      evidence: "REVIEW.md",
      evidenceSha256: sha256(reviewPath),
      error: status ? null : "REVIEW.md lacks Result: PASS | REVISE | BLOCKED.",
    };
  }
  const proofPath = join(sourceRoot, "proof.json");
  if (!existsSync(proofPath))
    return {
      error:
        "Cross-owner handoff requires REVIEW.md or proof.json review evidence.",
    };
  let proof;
  try {
    proof = JSON.parse(readFileSync(proofPath, "utf8"));
  } catch {
    return { error: "proof.json is not readable JSON review evidence." };
  }
  const status = proof?.review;
  return {
    status: status ? String(status) : undefined,
    reviewer:
      typeof proof?.reviewer === "string" ? proof.reviewer.trim() : undefined,
    reviewedDesignSha256:
      typeof proof?.reviewed_design_sha256 === "string"
        ? proof.reviewed_design_sha256
        : undefined,
    reviewedBriefSha256: proof?.reviewed_brief_sha256,
    reviewedSourceCompanions: reviewedSourceCompanionsFromProof(proof),
    evidence: "proof.json",
    evidenceSha256: sha256(proofPath),
    error: status ? null : "proof.json lacks a review value.",
  };
}

function reviewGateProblem(review, designSha256, briefSha256) {
  if (review.error) return review.error;
  if (review.status !== "PASS")
    return `Cross-owner handoff requires review PASS; received ${review.status}.`;
  if (!review.reviewer)
    return `${review.evidence} lacks a non-empty named reviewer.`;
  if (!/^[0-9a-f]{64}$/.test(review.reviewedDesignSha256 ?? ""))
    return `${review.evidence} lacks a valid Reviewed DESIGN.md SHA-256.`;
  if (review.reviewedDesignSha256 !== designSha256)
    return `${review.evidence} does not bind the current DESIGN.md SHA-256.`;
  if (review.reviewedBriefSha256 !== briefSha256)
    return `${review.evidence} does not bind the current BRIEF.md SHA-256.`;
  return null;
}

function reviewedSourceCompanionsFromProof(proof) {
  for (const candidate of [
    proof?.reviewed_source_companions,
    proof?.reviewed_companions,
  ]) {
    if (candidate && typeof candidate === "object" && !Array.isArray(candidate))
      return candidate;
  }
  return {};
}

function assertReviewedSourceCompanions(review, companions) {
  const selected = [...companions.files];
  for (const artifact of selected) {
    const reviewedHash =
      review.reviewedSourceCompanions?.[artifact.handoffPath];
    if (reviewedHash === artifact.sha256) continue;
    const problem = reviewedHash
      ? `${review.evidence} reviewed source companion ${artifact.handoffPath} at a different SHA-256.`
      : `${review.evidence} does not list selected source companion ${artifact.handoffPath} with its reviewed SHA-256.`;
    if (review.mode === "owner") waitingOwner(problem);
    fail(problem);
  }
  return selected.map((artifact) => ({
    path: artifact.handoffPath,
    sha256: artifact.sha256,
  }));
}

function normalizeIdentity(value) {
  return String(value ?? "")
    .replace(/\s+/g, " ")
    .trim();
}

function publicReview(review, reviewBoundary) {
  return {
    status: review.status,
    mode: review.mode,
    reviewOwner: review.reviewOwner,
    reviewer: review.reviewer,
    evidence: review.evidence,
    designSha256: review.reviewedDesignSha256,
    briefSha256: review.reviewedBriefSha256,
    reviewedSourceCompanions: reviewBoundary.reviewedSourceCompanions,
    deterministicDerivedExports: reviewBoundary.deterministicDerivedExports,
  };
}

function assertSafeOutput(outputPath, sourcePath) {
  if (isSameOrAncestor(outputPath, sourcePath))
    fail("Unsafe handoff output: selected source or an ancestor of it.");
}

function isSameOrAncestor(parent, child) {
  const scoped = relative(parent, child);
  return !scoped || (!scoped.startsWith("..") && !isAbsolute(scoped));
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function clean(value) {
  return String(value)
    .replace(/[\r\n]+/g, " ")
    .replace(/`/g, "'")
    .trim();
}

function handoffMarkdown({
  companions,
  manifest,
  portable,
  receivingOwner,
  reviewBoundary,
  source: sourcePath,
}) {
  const lines = [
    "# Portable design handoff",
    "",
    `Contract: \`${portable.contract}\``,
    "",
    `Handoff ID: \`${portable.id}\``,
    "",
    `Handoff revision: \`${portable.revision}\``,
    "",
    `Source: \`${clean(sourcePath)}\``,
    "",
    `Source revision: ${clean(portable.sourceRevision)}`,
    "",
    `Receiving owner: ${clean(receivingOwner)}`,
    "",
    `Receiving outcome: ${clean(portable.receivingOutcome)}`,
    "",
    `Review state: ${clean(portable.review.status)} (evidence: \`${clean(portable.review.evidence)}\`)`,
    "",
    `Review mode: ${clean(portable.review.mode)}`,
    "",
    `Review owner: ${clean(portable.review.reviewOwner)}`,
    "",
    `Reviewer: ${clean(portable.review.reviewer)}`,
    "",
    `Reviewed DESIGN.md SHA-256: \`${clean(portable.review.reviewedDesignSha256)}\``,
    "",
    `Reviewed BRIEF.md SHA-256: \`${clean(portable.review.reviewedBriefSha256)}\``,
    "",
    "Acceptance state: PENDING",
    "",
    "Accepted by: _(receiving owner must complete)_",
    "",
    "Accepted at: _(receiving owner must complete)_",
    "",
    "Acceptance statement: The receiving owner must explicitly replace the pending fields. Generation, copying, or use does not imply acceptance.",
    "",
    "## Canonical direction and ownership",
    "",
    "`DESIGN.md` is the required, portable, human-readable source of visual truth for this snapshot. It remains usable without the authoring package, original caller, or editor. `BRIEF.md`, the preview, tokens, exports, and assets provide context or implementation help; none replaces `DESIGN.md`.",
    "",
    "The design owner owns the visual direction, visual hierarchy, brand/style/voice expression, interaction and motion direction, and reusable visual assets represented by this snapshot until acceptance. The receiving owner becomes canonical for its implementation or production copy after explicit acceptance. Later design revisions do not update that accepted copy; they require a new handoff revision and re-acceptance.",
    "",
    "Content production owns the editorial thesis, scripts, media choices, editing, rendering, packaging, and publication. This snapshot invokes no other workflow.",
    "",
    "## Included snapshot and integrity",
    "",
    ...manifest.map(
      (item) => `- \`${item.path}\` — SHA-256 \`${item.sha256}\``,
    ),
    "",
    "`HANDOFF.md` is the human-readable binder and is excluded from its own integrity list.",
    "",
    "## Review and derivation boundary",
    "",
    `The named reviewer matches BRIEF.md Review owner ${clean(portable.review.reviewOwner)}. This Review identity is separate from receiving owner ${clean(receivingOwner)} and from the receiver's later acceptance decision.`,
    "",
    "The current `DESIGN.md` hash above is the reviewed canonical direction. Every pre-existing selected preview, asset, editable source, and native export must appear below with its exact reviewed source hash. CSS, design-token, and Tailwind files are instead deterministic derivatives generated from that exact reviewed `DESIGN.md`; they are integrity-hashed here but do not masquerade as pre-existing reviewed files.",
    "",
    "Reviewed source companions:",
    "",
    ...(reviewBoundary.reviewedSourceCompanions.length
      ? reviewBoundary.reviewedSourceCompanions.map(
          (item) => `- \`${item.path}\` — SHA-256 \`${item.sha256}\``,
        )
      : ["- None selected."]),
    "",
    "Deterministic derived exports:",
    "",
    ...(reviewBoundary.deterministicDerivedExports.length
      ? reviewBoundary.deterministicDerivedExports.map(
          (item) =>
            `- \`${item.path}\` — SHA-256 \`${item.sha256}\`; derived from reviewed DESIGN.md SHA-256 \`${item.derivedFromDesignSha256}\``,
        )
      : ["- None selected."]),
    "",
    "## Provenance and licensing",
    "",
    clean(portable.provenance),
    "",
    "## Known limitations",
    "",
    clean(portable.limitations),
    "",
    "The receiving owner must revalidate behavior, accessibility, content legibility, rights, and tokens against the receiving implementation or production surface.",
    "",
    "## Optional companions",
    "",
    "Preview HTML, assets and token/theme exports are included only when deliberately selected for this outcome. They never replace `DESIGN.md`; no receiving runtime depends on them.",
    "",
    ...(companions.preview
      ? ["- Preview: `index.html`"]
      : ["- Preview: not selected"]),
    ...companions.assets.map(
      (item) => `- Selected asset: \`${item.handoffPath}\``,
    ),
    ...companions.exports.map(
      (item) => `- Selected export: \`${item.handoffPath}\``,
    ),
    ...(companions.assets.length || companions.exports.length
      ? []
      : ["- Assets and token/theme exports: not selected"]),
    "",
  ];
  return `${lines.join("\n")}\n`;
}

function fail(message) {
  process.stderr.write(
    `${JSON.stringify({ success: false, error: message })}\n`,
  );
  process.exit(1);
}

function waitingOwner(message) {
  process.stderr.write(
    `${JSON.stringify({ success: false, status: "waiting-owner", error: message })}\n`,
  );
  process.exit(2);
}
