//! A project design guide (`DESIGN.md` / `design.md`) as bytes, and its one structured
//! design-values block: a pure, lossless codec (R19–R21 precursor; root `DESIGN.md`,
//! "Theme: supported and blocked").
//!
//! It only ever sees bytes a caller hands it and returns a snapshot, an inspection or a checked
//! proposal. It has no file system, path, metadata, host, MCP, browser, connection, timer or
//! canvas access: holding a [`Guide`] or a [`Revision`] grants nothing, and a [`Write`]
//! proposal is not a save. Reading and saving a project's real guide stay gated behind the
//! admitted, revocable source link (root `DESIGN.md`), which this module doesn't provide.
//!
//! The guide's content is data, never instructions: nothing in it is followed, fetched or run.
//!
//! **Bytes** ([`Guide::read`]): UTF-8 of at most [`MAX_BYTES`], an optional BOM, no NUL. The
//! exact bytes are kept, with every line's terminator (LF, CRLF, a lone CR, or none on a last
//! line); nothing is normalized.
//!
//! **Prose edits** ([`Guide::plan_text`]): the person's whole text (as an editor gives it, its
//! own terminators ignored) against the revision it was based on. Lines are matched by content
//! (a shortest line edit script, Myers' O(ND) diff, after the common first and last lines), so
//! every untouched line keeps its exact original bytes, also between two separate edits.
//! Terminator policy:
//! - an unchanged line keeps its own terminator;
//! - a new or changed line takes the terminator of the nearest original line before it, the
//!   lines it replaces included (so a rewritten line keeps the one it had), or at the very
//!   start the guide's first terminator, or LF in a guide without any;
//! - whether the guide ends with a terminator follows the draft (adding one uses the same
//!   rule);
//! - the BOM is the guide's: kept as it is (a leading U+FEFF in the draft stands for it);
//! - a CR and LF must never meet across two lines (read back as one CRLF, the second line
//!   would be lost, R63): a new empty line right after a line ending in a lone CR takes a lone
//!   CR, and a new line that would end in a lone CR right before an unchanged empty LF line
//!   takes CRLF. An unchanged line's bytes are never rewritten for this: where an edit would
//!   bring an unchanged empty LF line right after a lone CR, it is refused
//!   ([`PlanError::LineEndMerge`], nothing proposed).
//!
//! Every proposal is checked before it is returned: read back, its lines and final line end
//! must be exactly the draft's (else [`PlanError::SelfCheck`], nothing proposed).
//!
//! Working it out is bounded: where lines both go and come between the common first and last
//! lines, a change of more than [`MAX_CHANGED_LINES`] lines (inserted plus deleted), or one
//! needing more than [`MAX_DIFF_STEPS`] comparisons, is refused, never approximated. (Lines
//! only added, or only removed, there need no search.) So is a draft whose guide would be
//! larger than [`MAX_BYTES`], its BOM and kept terminators counted: every proposal is a guide
//! [`Guide::read`] takes.
//!
//! **Values** ([`Guide::tokens`]): exactly one fenced block whose info string is exactly `json`
//! (any letter case, nothing after it) and whose JSON is an object with `"opdDesignTokens": 1`
//! (the name also spelled with escapes). Recognized fences (supported subset of Markdown): a
//! line indented at most three spaces opening with three or more backticks or tildes, closed by
//! a line of the same character at least as long. Not recognized (R63: conservatively, without
//! a full Markdown parser): fences in block quotes; fences indented inside a list (after a list
//! item line, until a line at the margin after a blank line); anything inside an HTML block
//! (from a line starting with `<pre`, `<script`, `<style` or `<textarea` to its closing tag;
//! from `<!--`, `<![CDATA[`, `<?` or a declaration `<!` and a letter to its closing `-->`,
//! `]]>`, `?>` or `>`, blank lines inside included (R64), or unclosed to the guide's end; from
//! any other line starting with a tag or `<!` to the next blank line, so also a fence directly
//! after such a line); fences indented with a tab; indented code blocks. A fence-looking line
//! inside another fenced block is that block's content. Other fenced data (YAML, TOML, JSON
//! without the marker) is listed and left alone, never converted. Two candidate blocks, an
//! unclosed one, a marked block with another info string (`json custom-v2`), invalid JSON, a
//! repeated key (also one spelled with escapes), nesting deeper than [`MAX_DEPTH`] or another
//! version make the values read-only with the reason; prose edits stay possible. A `json` block
//! that doesn't parse counts as marked when the marker's name appears anywhere in it, each
//! character as itself or a `\u` escape, whatever its quotes (R64), so a broken marked block is
//! never taken for unmarked data beside a valid one; nothing is repaired or rewritten.
//!
//! Version 1 shape (a proposed shape for this codec, not an outside standard). Each group is an
//! object of named values; anything else in the block is kept as it is and listed as unknown:
//! - `colors`: `"#rrggbb"` or `"#rrggbbaa"` (sRGB, as the canvas stores colours);
//! - `fontSizes`: points, a number above 0 and at most 1000 (the renderer's text size);
//! - `fontWeights`: 400 (Regular) or 700 (Bold), the bundled Inter's weights; others stay
//!   visible as unsupported. No font family: the renderer draws Inter only;
//! - `spacing`, `radii`: a length, a number from 0 to 100000;
//! - `frames`: per name, an auto-layout's `direction` (`"horizontal"` or `"vertical"`), `gap`
//!   (a length) and `padding` (a length, or four: top, right, bottom, left).
//!
//! Units in strings (`"16px"`), other colour syntaxes and other weights are unsupported, shown
//! as such, never guessed. Nothing is read from prose. A number is used only when an f64 holds
//! it exactly as written (its shortest decimal is the same number: `16`, `0.5`, `4e2`,
//! `7.00e2`, `1.00e+1`); a spelling that would round (`400.00000000000000001`,
//! `1000.00000000000000001`, `-1e-99999`) stays unsupported, so nothing rounds into a supported
//! value or across a bound (R63).
//!
//! **Value edits** ([`Guide::plan_value`]): a known, supported value is replaced in its own byte
//! span only; every other byte (unknown keys, huge number lexemes, escapes, whitespace, prose)
//! stays. The block is never reformatted or written out from parsed values.

use paper_rs::model::Color;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Range;

/// The largest guide (and draft) accepted: 1 MiB.
pub const MAX_BYTES: usize = 1024 * 1024;
/// The prose diff's budgets: lines inserted plus deleted, and comparison steps.
pub const MAX_CHANGED_LINES: usize = 1000;
pub const MAX_DIFF_STEPS: usize = 20_000_000;
/// The deepest JSON nesting read.
pub const MAX_DEPTH: usize = 64;
/// The key that marks the supported block, and its one supported version.
pub const BLOCK_KEY: &str = "opdDesignTokens";
const BOM: &[u8] = b"\xEF\xBB\xBF";
const MAX_FONT_SIZE: f64 = 1000.0;
const MAX_LENGTH: f64 = 100_000.0;

/// How a line ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminator {
    Lf,
    CrLf,
    /// A lone carriage return.
    Cr,
    /// The last line, without one.
    None,
}

impl Terminator {
    fn bytes(self) -> &'static [u8] {
        match self {
            Terminator::Lf => b"\n",
            Terminator::CrLf => b"\r\n",
            Terminator::Cr => b"\r",
            Terminator::None => b"",
        }
    }
}

/// One line: its text is `start..end` of the guide's bytes, its terminator follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    pub end: usize,
    pub terminator: Terminator,
}

impl Line {
    /// Where the line ends, its terminator included.
    pub fn full_end(&self) -> usize {
        self.end + self.terminator.bytes().len()
    }
}

/// A guide's revision: its length and FNV-1a hash. It tells versions apart for a planned edit;
/// it is not a security check, and a later save must compare the bytes it replaces too.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Revision(String);

impl Revision {
    pub fn of(bytes: &[u8]) -> Revision {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Revision(format!("fnv1a64:{h:016x}:{}", bytes.len()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why bytes aren't a guide this codec takes. Nothing is read from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    TooLarge { len: usize, max: usize },
    Nul { offset: usize },
    NotUtf8 { offset: usize },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::TooLarge { len, max } => {
                write!(f, "the guide is {len} bytes; at most {max} are read")
            }
            ReadError::Nul { offset } => write!(f, "the guide has a NUL byte at {offset}"),
            ReadError::NotUtf8 { offset } => write!(f, "the guide isn't UTF-8 at byte {offset}"),
        }
    }
}

impl std::error::Error for ReadError {}

/// A planned change: nothing to write, or the new bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proposal {
    /// The result is byte-for-byte the guide as it is: nothing to write.
    Unchanged,
    Write(Write),
}

/// New bytes for a guide, planned against `base`. A proposal only; nothing is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    pub base: Revision,
    pub bytes: Vec<u8>,
    pub revision: Revision,
    /// The changed places, in order: `original` in the guide's bytes, `replacement` in `bytes`.
    /// Every byte outside them is the guide's own.
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub original: Range<usize>,
    pub replacement: Range<usize>,
}

/// Why a prose edit wasn't planned. Nothing is proposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The guide isn't the revision the draft was based on.
    Conflict {
        expected: Revision,
        actual: Revision,
    },
    /// The draft, or the guide it would make, is larger than [`MAX_BYTES`].
    TooLarge {
        len: usize,
        max: usize,
    },
    Nul {
        offset: usize,
    },
    /// The difference is too large to work out within the budgets.
    TooComplex,
    /// The planned bytes didn't read back as the draft (a defect; nothing is proposed).
    SelfCheck,
    /// The edit would bring an unchanged empty LF line right after a line ending in a lone CR:
    /// read back, that CR and LF would be one CRLF and the empty line would be lost, and
    /// avoiding it would mean rewriting a byte the person didn't change. Nothing is proposed.
    LineEndMerge,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanError::Conflict { expected, actual } => {
                write!(f, "the guide changed ({expected} expected, {actual} now)")
            }
            PlanError::TooLarge { len, max } => {
                write!(f, "it would be {len} bytes; at most {max} are taken")
            }
            PlanError::Nul { offset } => write!(f, "the text has a NUL character at {offset}"),
            PlanError::TooComplex => write!(
                f,
                "the change is too large to work out line by line (at most \
                 {MAX_CHANGED_LINES} lines changed); nothing was planned"
            ),
            PlanError::SelfCheck => {
                f.write_str("the plan didn't read back as the draft; nothing was planned")
            }
            PlanError::LineEndMerge => f.write_str(
                "an unchanged empty line would follow a lone CR and merge into it; nothing was \
                 planned",
            ),
        }
    }
}

impl std::error::Error for PlanError {}

/// A named-value group of version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Colors,
    FontSizes,
    FontWeights,
    Spacing,
    Radii,
    Frames,
}

impl Group {
    pub const ALL: [Group; 6] = [
        Group::Colors,
        Group::FontSizes,
        Group::FontWeights,
        Group::Spacing,
        Group::Radii,
        Group::Frames,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Group::Colors => "colors",
            Group::FontSizes => "fontSizes",
            Group::FontWeights => "fontWeights",
            Group::Spacing => "spacing",
            Group::Radii => "radii",
            Group::Frames => "frames",
        }
    }

    fn from_key(key: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.key() == key)
    }
}

/// A frame layout's member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameField {
    Direction,
    Gap,
    Padding,
}

impl FrameField {
    pub fn key(self) -> &'static str {
        match self {
            FrameField::Direction => "direction",
            FrameField::Gap => "gap",
            FrameField::Padding => "padding",
        }
    }

    fn from_key(key: &str) -> Option<FrameField> {
        [FrameField::Direction, FrameField::Gap, FrameField::Padding]
            .into_iter()
            .find(|f| f.key() == key)
    }
}

/// An auto-layout direction, as the canvas names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// A supported value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TokenValue {
    Color(Color),
    /// A text size in points.
    Size(f64),
    /// 400 or 700.
    Weight(u16),
    /// Spacing, a radius or a frame gap.
    Length(f64),
    Direction(Axis),
    /// Top, right, bottom, left.
    Padding([f64; 4]),
}

/// Where a value is: `group.name`, or `frames.name.field`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValuePath {
    pub group: Group,
    pub name: String,
    pub field: Option<FrameField>,
}

impl fmt::Display for ValuePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.group.key(), self.name)?;
        if let Some(field) = self.field {
            write!(f, ".{}", field.key())?;
        }
        Ok(())
    }
}

/// A fenced block: its lines (fence lines included) and info string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FencedBlock {
    pub lines: Range<usize>,
    pub info: String,
}

/// Why the values can't be edited (they stay as they are; prose edits still work).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadOnly {
    /// More than one block is marked `opdDesignTokens`.
    Multiple,
    /// The marked block has no closing fence.
    Unclosed,
    /// The marked block's info string isn't exactly `json` (`json custom-v2`, say).
    Format {
        info: String,
    },
    /// The marked block isn't valid JSON; `offset` is in the guide's bytes.
    Invalid {
        message: String,
        offset: usize,
    },
    /// An object has `key` twice (also when spelled with escapes); `path` is the object's.
    DuplicateKey {
        path: String,
        key: String,
    },
    TooDeep,
    /// `opdDesignTokens` isn't `1`.
    Version {
        lexeme: String,
    },
}

impl fmt::Display for ReadOnly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadOnly::Multiple => f.write_str(
                "more than one block is marked opdDesignTokens, so it isn't clear which one holds \
                 the values",
            ),
            ReadOnly::Unclosed => f.write_str("the opdDesignTokens block has no closing fence"),
            ReadOnly::Format { info } => write!(
                f,
                "the opdDesignTokens block is fenced as {info:?}, not plain json, so its format \
                 isn't one this editor supports"
            ),
            ReadOnly::Invalid { message, offset } => {
                write!(
                    f,
                    "the opdDesignTokens block isn't valid JSON: {message} (byte {offset})"
                )
            }
            ReadOnly::DuplicateKey { path, key } => {
                let at = if path.is_empty() { "the block" } else { path };
                write!(f, "{at} has the name {key:?} more than once")
            }
            ReadOnly::TooDeep => write!(f, "the block nests deeper than {MAX_DEPTH} levels"),
            ReadOnly::Version { lexeme } => {
                write!(
                    f,
                    "opdDesignTokens {lexeme} isn't a version this editor supports (1)"
                )
            }
        }
    }
}

/// What a guide holds in values.
#[derive(Debug, Clone, PartialEq)]
pub enum Tokens {
    /// No block marked `opdDesignTokens`: an ordinary guide (nothing is created). Other fenced
    /// data blocks (JSON, YAML, TOML) are listed, unsupported and left alone.
    None {
        other_blocks: Vec<FencedBlock>,
    },
    Supported(TokenBlock),
    /// Values present but not editable here; `blocks` are the marked ones.
    ReadOnly {
        reason: ReadOnly,
        blocks: Vec<FencedBlock>,
    },
}

/// The supported block's values.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenBlock {
    pub block: FencedBlock,
    pub entries: Vec<Entry>,
    /// Members the codec doesn't read (dotted paths), kept as they are.
    pub unknown: Vec<String>,
}

impl TokenBlock {
    pub fn get(&self, path: &ValuePath) -> Option<&Entry> {
        self.entries.iter().find(|e| &e.path == path)
    }
}

/// A named value and its source span in the guide's bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: ValuePath,
    pub span: Range<usize>,
    pub state: EntryState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntryState {
    Supported(TokenValue),
    /// Shown, kept, not usable here; why.
    Unsupported(&'static str),
}

/// Why a value edit wasn't planned. Nothing is proposed.
#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    Conflict {
        expected: Revision,
        actual: Revision,
    },
    /// The values aren't editable (no block, or read-only); why.
    NotEditable { reason: String },
    /// No such value in the block (this edits known values only).
    NoSuchValue(ValuePath),
    /// The value there is unsupported; it isn't converted.
    Unsupported {
        path: ValuePath,
        reason: &'static str,
    },
    /// The new value isn't one this place takes.
    Rejected { reason: &'static str },
    /// The planned bytes didn't read back as the edit (a defect; nothing is proposed).
    SelfCheck,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::Conflict { expected, actual } => {
                write!(f, "the guide changed ({expected} expected, {actual} now)")
            }
            EditError::NotEditable { reason } => write!(f, "the values can't be edited: {reason}"),
            EditError::NoSuchValue(p) => write!(f, "the guide has no value {p}"),
            EditError::Unsupported { path, reason } => {
                write!(
                    f,
                    "{path} is unsupported here ({reason}); it is kept as it is"
                )
            }
            EditError::Rejected { reason } => write!(f, "the new value isn't taken: {reason}"),
            EditError::SelfCheck => f.write_str("the edit didn't check out; nothing was planned"),
        }
    }
}

impl std::error::Error for EditError {}

/// A guide's exact bytes, read.
#[derive(Debug, Clone)]
pub struct Guide {
    text: String,
    bom: bool,
    lines: Vec<Line>,
    revision: Revision,
}

impl Guide {
    /// Take `bytes` as a guide: refused if larger than [`MAX_BYTES`], with a NUL, or not UTF-8.
    pub fn read(bytes: &[u8]) -> Result<Guide, ReadError> {
        if bytes.len() > MAX_BYTES {
            return Err(ReadError::TooLarge {
                len: bytes.len(),
                max: MAX_BYTES,
            });
        }
        if let Some(offset) = bytes.iter().position(|&b| b == 0) {
            return Err(ReadError::Nul { offset });
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|e| ReadError::NotUtf8 {
                offset: e.valid_up_to(),
            })?
            .to_string();
        let bom = bytes.starts_with(BOM);
        let lines = split_lines(&text, if bom { BOM.len() } else { 0 });
        Ok(Guide {
            revision: Revision::of(bytes),
            text,
            bom,
            lines,
        })
    }

    /// The exact bytes.
    pub fn bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }

    pub fn revision(&self) -> &Revision {
        &self.revision
    }

    pub fn has_bom(&self) -> bool {
        self.bom
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// A line's text, without its terminator.
    pub fn line_text(&self, i: usize) -> &str {
        let l = &self.lines[i];
        &self.text[l.start..l.end]
    }

    /// Whether line `i` is empty and ends in LF.
    fn is_blank_lf(&self, i: usize) -> bool {
        let l = &self.lines[i];
        l.start == l.end && l.terminator == Terminator::Lf
    }

    fn body_start(&self) -> usize {
        if self.bom { BOM.len() } else { 0 }
    }

    /// Plan the prose edit that makes the guide read `draft` (see the module's terminator
    /// policy), against the revision `expected` it was based on.
    pub fn plan_text(&self, expected: &Revision, draft: &str) -> Result<Proposal, PlanError> {
        if expected != &self.revision {
            return Err(PlanError::Conflict {
                expected: expected.clone(),
                actual: self.revision.clone(),
            });
        }
        if draft.len() > MAX_BYTES {
            return Err(PlanError::TooLarge {
                len: draft.len(),
                max: MAX_BYTES,
            });
        }
        if let Some(offset) = draft.bytes().position(|b| b == 0) {
            return Err(PlanError::Nul { offset });
        }
        let draft = draft.strip_prefix('\u{feff}').unwrap_or(draft);
        let new_lines = split_lines(draft, 0);
        let ends_with_newline = new_lines
            .last()
            .is_some_and(|l| l.terminator != Terminator::None);
        let a: Vec<&str> = (0..self.lines.len()).map(|i| self.line_text(i)).collect();
        let b: Vec<&str> = new_lines.iter().map(|l| &draft[l.start..l.end]).collect();
        let ops = diff(&a, &b)?;

        // Which terminator each new line takes, from the original lines passed so far.
        let mut term = self
            .lines
            .iter()
            .map(|l| l.terminator)
            .find(|t| *t != Terminator::None)
            .unwrap_or(Terminator::Lf);
        let mut items = Vec::with_capacity(ops.len());
        for op in ops {
            match op {
                Op::Keep(i) | Op::Delete(i) => {
                    items.push((op, term));
                    if self.lines[i].terminator != Terminator::None {
                        term = self.lines[i].terminator;
                    }
                }
                Op::Insert(_) => items.push((op, term)),
            }
        }
        let last_emitted = items
            .iter()
            .rposition(|(op, _)| !matches!(op, Op::Delete(_)));

        let mut out = Vec::with_capacity(self.text.len() + draft.len());
        out.extend_from_slice(&self.bytes()[..self.body_start()]);
        let mut orig_pos = self.body_start();
        let mut hunks: Vec<Hunk> = Vec::new();
        // The last line written ends in a lone CR: an empty line after it must not end in LF,
        // which would read as one CRLF and lose that empty line (R63).
        let mut after_cr = false;
        // For each step: whether the next line written is an unchanged empty LF line.
        let mut before_blank_lf = vec![false; items.len()];
        let mut next = false;
        for (k, (op, _)) in items.iter().enumerate().rev() {
            before_blank_lf[k] = next;
            match op {
                Op::Keep(i) => next = self.is_blank_lf(*i),
                Op::Insert(_) => next = false,
                Op::Delete(_) => {}
            }
        }
        for (k, (op, policy)) in items.into_iter().enumerate() {
            let last = Some(k) == last_emitted;
            match op {
                Op::Delete(i) => {
                    let l = &self.lines[i];
                    push_hunk(&mut hunks, l.start..l.full_end(), out.len()..out.len());
                    orig_pos = l.full_end();
                }
                Op::Keep(i) => {
                    let l = &self.lines[i];
                    let want = match (last, ends_with_newline, l.terminator) {
                        (true, false, _) => Terminator::None,
                        (_, _, Terminator::None) => policy,
                        (_, _, t) => t,
                    };
                    if after_cr && self.is_blank_lf(i) {
                        // Its LF would complete the CR before it: the line would be lost, and
                        // its own bytes are never rewritten to avoid that.
                        return Err(PlanError::LineEndMerge);
                    }
                    after_cr = want == Terminator::Cr;
                    let at = out.len();
                    if want == l.terminator {
                        out.extend_from_slice(&self.bytes()[l.start..l.full_end()]);
                    } else {
                        out.extend_from_slice(&self.bytes()[l.start..l.end]);
                        out.extend_from_slice(want.bytes());
                        push_hunk(&mut hunks, l.start..l.full_end(), at..out.len());
                    }
                    orig_pos = l.full_end();
                }
                Op::Insert(j) => {
                    let mut want = if last && !ends_with_newline {
                        Terminator::None
                    } else {
                        policy
                    };
                    if after_cr && b[j].is_empty() && want == Terminator::Lf {
                        want = Terminator::Cr;
                    }
                    // A lone CR right before an unchanged empty LF line would take its LF.
                    if want == Terminator::Cr && before_blank_lf[k] {
                        want = Terminator::CrLf;
                    }
                    after_cr = want == Terminator::Cr;
                    let at = out.len();
                    out.extend_from_slice(b[j].as_bytes());
                    out.extend_from_slice(want.bytes());
                    push_hunk(&mut hunks, orig_pos..orig_pos, at..out.len());
                }
            }
        }
        // Read back, the result must be the draft: its lines and its final line end.
        let body =
            std::str::from_utf8(&out[self.body_start()..]).map_err(|_| PlanError::SelfCheck)?;
        let back = split_lines(body, 0);
        let same = back.len() == b.len()
            && back
                .iter()
                .zip(&b)
                .all(|(l, s)| body[l.start..l.end] == **s)
            && back
                .last()
                .is_some_and(|l| l.terminator != Terminator::None)
                == ends_with_newline;
        if !same {
            return Err(PlanError::SelfCheck);
        }
        if out == self.bytes() {
            return Ok(Proposal::Unchanged);
        }
        // The guide it would make must be one `Guide::read` takes (run149: a MAX_BYTES draft
        // over a CRLF guide made MAX_BYTES + 1 bytes). Refused whole; nothing is shortened or
        // normalized to fit.
        if out.len() > MAX_BYTES {
            return Err(PlanError::TooLarge {
                len: out.len(),
                max: MAX_BYTES,
            });
        }
        Ok(Proposal::Write(Write {
            base: self.revision.clone(),
            revision: Revision::of(&out),
            bytes: out,
            hunks,
        }))
    }

    /// The guide's values (see the module's version 1 shape).
    pub fn tokens(&self) -> Tokens {
        let fences = self.fences();
        let mut candidates = Vec::new();
        let mut other_blocks = Vec::new();
        // A fence inside a list or HTML block is an example, not a block of this guide's (R63).
        for f in fences.iter().filter(|f| !f.contained) {
            let block = FencedBlock {
                lines: f.open..f.close.map_or(self.lines.len(), |c| c + 1),
                info: f.info.clone(),
            };
            let first = f.info.split_whitespace().next().unwrap_or("");
            let content = self.fence_content(f);
            let text = &self.text[content.clone()];
            let parsed = first.eq_ignore_ascii_case("json").then(|| parse(text));
            // A block that doesn't parse is marked when the marker appears in it at all, also
            // spelled with escapes (R63) and whatever its quotes (R64), so a broken marked block
            // is never unmarked data.
            let marked = match &parsed {
                Some(Ok(Json::Object(members, _))) => members.iter().any(|m| m.key == BLOCK_KEY),
                Some(Err(_)) => mentions_marker(text),
                _ => false,
            };
            if marked {
                let exact = f.info.eq_ignore_ascii_case("json");
                candidates.push((f.close.is_some(), block, content.start, parsed, exact));
            } else if ["json", "jsonc", "json5", "yaml", "yml", "toml"]
                .iter()
                .any(|k| first.eq_ignore_ascii_case(k))
            {
                other_blocks.push(block);
            }
        }
        let blocks: Vec<FencedBlock> = candidates.iter().map(|c| c.1.clone()).collect();
        let read_only = |reason| Tokens::ReadOnly {
            reason,
            blocks: blocks.clone(),
        };
        let (closed, block, base, parsed, exact) = match candidates.len() {
            0 => return Tokens::None { other_blocks },
            1 => candidates.remove(0),
            _ => return read_only(ReadOnly::Multiple),
        };
        if !closed {
            return read_only(ReadOnly::Unclosed);
        }
        if !exact {
            return read_only(ReadOnly::Format { info: block.info });
        }
        let root = match parsed {
            Some(Ok(root)) => root,
            Some(Err(JsonProblem::Invalid { message, offset })) => {
                return read_only(ReadOnly::Invalid {
                    message,
                    offset: base + offset,
                });
            }
            Some(Err(JsonProblem::DuplicateKey { path, key })) => {
                return read_only(ReadOnly::DuplicateKey { path, key });
            }
            Some(Err(JsonProblem::TooDeep)) => return read_only(ReadOnly::TooDeep),
            None => return Tokens::None { other_blocks },
        };
        let Json::Object(members, _) = root else {
            return Tokens::None { other_blocks };
        };
        let src = &self.text[base..];
        let mut entries = Vec::new();
        let mut unknown = Vec::new();
        for m in &members {
            if m.key == BLOCK_KEY {
                match &m.value {
                    Json::Number(r) if &src[r.clone()] == "1" => {}
                    v => {
                        let r = v.span();
                        return read_only(ReadOnly::Version {
                            lexeme: src[r].to_string(),
                        });
                    }
                }
                continue;
            }
            let Some(group) = Group::from_key(&m.key) else {
                unknown.push(m.key.clone());
                continue;
            };
            let Json::Object(named, _) = &m.value else {
                unknown.push(m.key.clone());
                continue;
            };
            for n in named {
                let at = |r: Range<usize>| base + r.start..base + r.end;
                if group != Group::Frames {
                    entries.push(Entry {
                        path: ValuePath {
                            group,
                            name: n.key.clone(),
                            field: None,
                        },
                        span: at(n.value.span()),
                        state: classify(group, None, &n.value, src),
                    });
                    continue;
                }
                let Json::Object(fields, _) = &n.value else {
                    unknown.push(format!("{}.{}", m.key, n.key));
                    continue;
                };
                for fm in fields {
                    let Some(field) = FrameField::from_key(&fm.key) else {
                        unknown.push(format!("{}.{}.{}", m.key, n.key, fm.key));
                        continue;
                    };
                    entries.push(Entry {
                        path: ValuePath {
                            group,
                            name: n.key.clone(),
                            field: Some(field),
                        },
                        span: at(fm.value.span()),
                        state: classify(group, Some(field), &fm.value, src),
                    });
                }
            }
        }
        Tokens::Supported(TokenBlock {
            block,
            entries,
            unknown,
        })
    }

    /// Plan changing the known, supported value at `path` to `value`, in its own byte span only,
    /// against the revision `expected` it was based on.
    pub fn plan_value(
        &self,
        expected: &Revision,
        path: &ValuePath,
        value: &TokenValue,
    ) -> Result<Proposal, EditError> {
        if expected != &self.revision {
            return Err(EditError::Conflict {
                expected: expected.clone(),
                actual: self.revision.clone(),
            });
        }
        let block = match self.tokens() {
            Tokens::Supported(b) => b,
            Tokens::None { .. } => {
                return Err(EditError::NotEditable {
                    reason: "the guide has no opdDesignTokens block".to_string(),
                });
            }
            Tokens::ReadOnly { reason, .. } => {
                return Err(EditError::NotEditable {
                    reason: reason.to_string(),
                });
            }
        };
        let entry = block
            .get(path)
            .ok_or_else(|| EditError::NoSuchValue(path.clone()))?;
        let current = match entry.state {
            EntryState::Supported(v) => v,
            EntryState::Unsupported(reason) => {
                return Err(EditError::Unsupported {
                    path: path.clone(),
                    reason,
                });
            }
        };
        accepts(path, value).map_err(|reason| EditError::Rejected { reason })?;
        if current == *value {
            return Ok(Proposal::Unchanged);
        }
        let lexeme = render(value);
        let span = entry.span.clone();
        let mut out = Vec::with_capacity(self.text.len() + lexeme.len());
        out.extend_from_slice(&self.bytes()[..span.start]);
        out.extend_from_slice(lexeme.as_bytes());
        out.extend_from_slice(&self.bytes()[span.end..]);
        // The planned bytes must read back as exactly this edit.
        let back = Guide::read(&out).map_err(|_| EditError::SelfCheck)?;
        match back.tokens() {
            Tokens::Supported(b)
                if b.get(path).map(|e| &e.state) == Some(&EntryState::Supported(*value)) => {}
            _ => return Err(EditError::SelfCheck),
        }
        Ok(Proposal::Write(Write {
            base: self.revision.clone(),
            revision: back.revision.clone(),
            hunks: vec![Hunk {
                original: span.clone(),
                replacement: span.start..span.start + lexeme.len(),
            }],
            bytes: out,
        }))
    }

    /// The fenced blocks, in order (see the module's supported subset). Lines of an HTML block
    /// are never read as fences; a fence indented inside a list is `contained` (an example).
    fn fences(&self) -> Vec<Fence> {
        let mut out = Vec::new();
        let mut html: Option<HtmlEnd> = None;
        let mut list = false;
        let mut after_blank = false;
        let mut i = 0;
        while i < self.lines.len() {
            let line = self.line_text(i);
            if let Some(end) = html {
                if html_ends(end, line) {
                    html = None;
                }
                after_blank = is_blank(line);
                i += 1;
                continue;
            }
            if let Some((ch, len, info)) = fence_open(line) {
                let close =
                    (i + 1..self.lines.len()).find(|&k| fence_close(self.line_text(k), ch, len));
                let contained = list && indent(line) > 0;
                out.push(Fence {
                    open: i,
                    close,
                    info: info.to_string(),
                    contained,
                });
                // A fence at the margin ends a list.
                list = list && contained;
                after_blank = false;
                i = close.map_or(self.lines.len(), |c| c + 1);
                continue;
            }
            if is_blank(line) {
                after_blank = true;
                i += 1;
                continue;
            }
            if let Some(end) = html_start(line) {
                if end == HtmlEnd::Blank || !html_ends(end, line) {
                    html = Some(end);
                }
            } else if list_item(line) {
                list = true;
            } else if after_blank && indent(line) == 0 {
                list = false;
            }
            after_blank = false;
            i += 1;
        }
        out
    }

    /// A fenced block's content: after its opening line, up to its closing line (or the end).
    fn fence_content(&self, f: &Fence) -> Range<usize> {
        let start = self.lines[f.open].full_end();
        let end = f.close.map_or(self.text.len(), |c| self.lines[c].start);
        start..end
    }
}

/// Add a changed place, joined to the last one when they touch.
fn push_hunk(hunks: &mut Vec<Hunk>, original: Range<usize>, replacement: Range<usize>) {
    match hunks.last_mut() {
        Some(h) if h.original.end == original.start && h.replacement.end == replacement.start => {
            h.original.end = original.end;
            h.replacement.end = replacement.end;
        }
        _ => hunks.push(Hunk {
            original,
            replacement,
        }),
    }
}

struct Fence {
    open: usize,
    close: Option<usize>,
    info: String,
    /// Indented inside a list: an example, not one of the guide's blocks.
    contained: bool,
}

/// How an HTML block ends: at a line containing its closing tag, at a line containing a text,
/// or at a blank line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HtmlEnd {
    Tag(&'static str),
    Text(&'static str),
    Blank,
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_blank(line: &str) -> bool {
    line.trim_matches(|c| c == ' ' || c == '\t').is_empty()
}

/// The HTML block a line starts, if any: indented at most three spaces, `<` then a tag name,
/// `/`, `!` or `?` (a conservative reading of CommonMark's HTML blocks). A comment, CDATA
/// section, processing instruction or declaration runs to its closing `-->`, `]]>`, `?>` or `>`
/// (unclosed, to the guide's end); another tag, to a blank line.
fn html_start(line: &str) -> Option<HtmlEnd> {
    if indent(line) > 3 {
        return None;
    }
    let rest = line.trim_start_matches(' ').to_ascii_lowercase();
    let after = rest.strip_prefix('<')?;
    for tag in ["pre", "script", "style", "textarea"] {
        if let Some(next) = after.strip_prefix(tag)
            && (next.is_empty() || next.starts_with([' ', '\t', '>']))
        {
            return Some(HtmlEnd::Tag(tag));
        }
    }
    // The enclosed forms end at their own closing text only, never at a blank line inside them.
    if after.starts_with("!--") {
        return Some(HtmlEnd::Text("-->"));
    }
    if after.starts_with("![cdata[") {
        return Some(HtmlEnd::Text("]]>"));
    }
    if after.starts_with('?') {
        return Some(HtmlEnd::Text("?>"));
    }
    if after.starts_with('!') && after.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic) {
        return Some(HtmlEnd::Text(">"));
    }
    let next = *after.as_bytes().first()?;
    (next.is_ascii_alphabetic() || matches!(next, b'/' | b'!')).then_some(HtmlEnd::Blank)
}

/// Whether `line` ends an HTML block that ends at `end`.
fn html_ends(end: HtmlEnd, line: &str) -> bool {
    match end {
        HtmlEnd::Tag(tag) => line.to_ascii_lowercase().contains(&format!("</{tag}>")),
        HtmlEnd::Text(text) => line.contains(text),
        HtmlEnd::Blank => is_blank(line),
    }
}

/// A list item's first line: indented at most three spaces, `-`, `+` or `*`, or one to nine
/// digits and `.` or `)`, then a space, a tab or the line's end.
fn list_item(line: &str) -> bool {
    if indent(line) > 3 {
        return false;
    }
    let rest = line.trim_start_matches(' ').as_bytes();
    let marker = match rest.first() {
        Some(b'-' | b'+' | b'*') => 1,
        Some(b'0'..=b'9') => {
            let n = rest.iter().take_while(|b| b.is_ascii_digit()).count();
            if n > 9 || !matches!(rest.get(n), Some(b'.' | b')')) {
                return false;
            }
            n + 1
        }
        _ => return false,
    };
    matches!(rest.get(marker), None | Some(b' ' | b'\t'))
}

/// Whether the marker's name appears anywhere in `text` (not valid JSON), each character as
/// itself or as a `\u` escape. Quotes aren't paired (a broken string before the name must not
/// hide it, R64), and each place reads at most six bytes a character, so the scan is linear.
fn mentions_marker(text: &str) -> bool {
    let s = text.as_bytes();
    (0..s.len()).any(|i| marker_at(s, i))
}

/// Whether the marker's name, each character as itself or as a `\u` escape, starts at `i`.
fn marker_at(s: &[u8], mut i: usize) -> bool {
    for want in BLOCK_KEY.bytes() {
        if s.get(i) == Some(&want) {
            i += 1;
            continue;
        }
        if s.get(i..i + 2) != Some(&b"\\u"[..]) {
            return false;
        }
        let Some(hex) = s.get(i + 2..i + 6) else {
            return false;
        };
        let value = hex
            .iter()
            .try_fold(0, |v, &h| Some(v * 16 + char::from(h).to_digit(16)?));
        if value != Some(u32::from(want)) {
            return false;
        }
        i += 6;
    }
    true
}

/// An opening fence line: its character, length and info string.
fn fence_open(line: &str) -> Option<(u8, usize, &str)> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let n = rest.bytes().take_while(|&c| c == ch).count();
    if n < 3 {
        return None;
    }
    let info = rest[n..].trim_matches(|c| c == ' ' || c == '\t');
    if ch == b'`' && info.contains('`') {
        return None;
    }
    Some((ch, n, info))
}

/// Whether `line` closes a fence of `len` `ch` characters.
fn fence_close(line: &str, ch: u8, len: usize) -> bool {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return false;
    }
    let n = rest.bytes().take_while(|&c| c == ch).count();
    n >= len && rest[n..].trim_matches(|c| c == ' ' || c == '\t').is_empty()
}

/// The lines of `text` from byte `start`, each with its terminator.
fn split_lines(text: &str, start: usize) -> Vec<Line> {
    let b = text.as_bytes();
    let mut lines = Vec::new();
    let mut line_start = start;
    let mut i = start;
    while i < b.len() {
        let (terminator, len) = match b[i] {
            b'\n' => (Terminator::Lf, 1),
            b'\r' if b.get(i + 1) == Some(&b'\n') => (Terminator::CrLf, 2),
            b'\r' => (Terminator::Cr, 1),
            _ => {
                i += 1;
                continue;
            }
        };
        lines.push(Line {
            start: line_start,
            end: i,
            terminator,
        });
        i += len;
        line_start = i;
    }
    if line_start < b.len() {
        lines.push(Line {
            start: line_start,
            end: b.len(),
            terminator: Terminator::None,
        });
    }
    lines
}

/// One step of the line alignment: keep or delete original line `i`, insert draft line `j`.
#[derive(Debug, Clone, Copy)]
enum Op {
    Keep(usize),
    Delete(usize),
    Insert(usize),
}

/// The alignment of `a` (original lines) to `b` (draft lines): the common first and last
/// lines, then a shortest edit script of the rest (see [`myers`]). Within each run of changes
/// the deletions come first, so a new line follows the lines it replaces.
fn diff(a: &[&str], b: &[&str]) -> Result<Vec<Op>, PlanError> {
    let mut ops = align(a, b)?;
    let mut k = 0;
    while k < ops.len() {
        let end = (k..ops.len())
            .find(|&e| matches!(ops[e], Op::Keep(_)))
            .unwrap_or(ops.len());
        ops[k..end].sort_by_key(|op| matches!(op, Op::Insert(_)));
        k = end + 1;
    }
    Ok(ops)
}

fn align(a: &[&str], b: &[&str]) -> Result<Vec<Op>, PlanError> {
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf]
    {
        suf += 1;
    }
    let (n, m) = (a.len() - pre - suf, b.len() - pre - suf);
    let mut ops: Vec<Op> = (0..pre).map(Op::Keep).collect();
    if n == 0 || m == 0 {
        ops.extend((0..n).map(|i| Op::Delete(pre + i)));
        ops.extend((0..m).map(|j| Op::Insert(pre + j)));
    } else {
        // Lines as small ids, so the search compares numbers.
        let mut ids: HashMap<&str, u32> = HashMap::new();
        let mut ai = Vec::with_capacity(n);
        for &s in &a[pre..pre + n] {
            let next = ids.len() as u32;
            ai.push(*ids.entry(s).or_insert(next));
        }
        let mut bi = Vec::with_capacity(m);
        for &s in &b[pre..pre + m] {
            let next = ids.len() as u32;
            bi.push(*ids.entry(s).or_insert(next));
        }
        ops.extend(myers(&ai, &bi)?.into_iter().map(|op| match op {
            Op::Keep(i) => Op::Keep(pre + i),
            Op::Delete(i) => Op::Delete(pre + i),
            Op::Insert(j) => Op::Insert(pre + j),
        }));
    }
    ops.extend((a.len() - suf..a.len()).map(Op::Keep));
    Ok(ops)
}

/// A shortest edit script from `a` to `b` (Myers, "An O(ND) Difference Algorithm"), refused
/// beyond [`MAX_CHANGED_LINES`] changes or [`MAX_DIFF_STEPS`] steps. Its cost grows with the
/// number of changes, not the guide's length, so two edits far apart in a long guide are cheap.
fn myers(a: &[u32], b: &[u32]) -> Result<Vec<Op>, PlanError> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max_d = (a.len() + b.len()).min(MAX_CHANGED_LINES) as isize;
    let off = max_d + 1;
    let at = |k: isize| (off + k) as usize;
    // `v[at(k)]`: the furthest x reached on diagonal k (y = x - k).
    let mut v = vec![0isize; at(off) + 1];
    // What `v` held for -d..=d before each round d, for the way back.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut steps = 0usize;
    for d in 0..=max_d {
        trace.push(v[at(-d)..=at(d)].to_vec());
        let mut k = -d;
        while k <= d {
            let down = k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]);
            let mut x = if down { v[at(k + 1)] } else { v[at(k - 1)] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
                steps += 1;
            }
            steps += 1;
            if steps > MAX_DIFF_STEPS {
                return Err(PlanError::TooComplex);
            }
            v[at(k)] = x;
            if x >= n && y >= m {
                return Ok(myers_path(&trace, d, n, m));
            }
            k += 2;
        }
    }
    Err(PlanError::TooComplex)
}

/// The edit script [`myers`] found, from its `trace`, from (n, m) back to the start.
fn myers_path(trace: &[Vec<isize>], end: isize, n: isize, m: isize) -> Vec<Op> {
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=end).rev() {
        let before = &trace[d as usize];
        let get = |k: isize| before[(k + d) as usize];
        let k = x - y;
        let down = k == -d || (k != d && get(k - 1) < get(k + 1));
        let prev_k = if down { k + 1 } else { k - 1 };
        let prev_x = get(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            ops.push(Op::Keep((x - 1) as usize));
            x -= 1;
            y -= 1;
        }
        if down {
            ops.push(Op::Insert((y - 1) as usize));
        } else {
            ops.push(Op::Delete((x - 1) as usize));
        }
        x = prev_x;
        y = prev_y;
    }
    while x > 0 && y > 0 {
        ops.push(Op::Keep((x - 1) as usize));
        x -= 1;
        y -= 1;
    }
    ops.reverse();
    ops
}

// ---- JSON, read with the byte span of every value (nothing is written back from it)

#[derive(Debug, Clone)]
enum Json {
    Object(Vec<Member>, Range<usize>),
    Array(Vec<Json>, Range<usize>),
    String(String, Range<usize>),
    Number(Range<usize>),
    /// `true`, `false` or `null`.
    Literal(Range<usize>),
}

#[derive(Debug, Clone)]
struct Member {
    key: String,
    value: Json,
}

impl Json {
    fn span(&self) -> Range<usize> {
        match self {
            Json::Object(_, r)
            | Json::Array(_, r)
            | Json::String(_, r)
            | Json::Number(r)
            | Json::Literal(r) => r.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JsonProblem {
    Invalid { message: String, offset: usize },
    DuplicateKey { path: String, key: String },
    TooDeep,
}

/// One JSON value (RFC 8259) and nothing but whitespace around it.
fn parse(text: &str) -> Result<Json, JsonProblem> {
    let mut p = Parser {
        s: text.as_bytes(),
        src: text,
        i: 0,
    };
    let v = p.value(0, "")?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("more follows the JSON value"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    src: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn err(&self, message: &str) -> JsonProblem {
        JsonProblem::Invalid {
            message: message.to_string(),
            offset: self.i,
        }
    }

    fn value(&mut self, depth: usize, path: &str) -> Result<Json, JsonProblem> {
        if depth > MAX_DEPTH {
            return Err(JsonProblem::TooDeep);
        }
        self.ws();
        match self.peek() {
            None => Err(self.err("a value was expected")),
            Some(b'{') => self.object(depth, path),
            Some(b'[') => self.array(depth, path),
            Some(b'"') => {
                let (s, r) = self.string()?;
                Ok(Json::String(s, r))
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => self.literal(),
        }
    }

    fn object(&mut self, depth: usize, path: &str) -> Result<Json, JsonProblem> {
        let start = self.i;
        self.i += 1;
        let mut members = Vec::new();
        let mut seen = HashSet::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Json::Object(members, start..self.i));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err("a member name (a string) was expected"));
            }
            let (key, _) = self.string()?;
            if !seen.insert(key.clone()) {
                return Err(JsonProblem::DuplicateKey {
                    path: path.to_string(),
                    key,
                });
            }
            self.ws();
            if self.peek() != Some(b':') {
                return Err(self.err("':' was expected"));
            }
            self.i += 1;
            let child = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            let value = self.value(depth + 1, &child)?;
            members.push(Member { key, value });
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Json::Object(members, start..self.i));
                }
                _ => return Err(self.err("',' or '}' was expected")),
            }
        }
    }

    fn array(&mut self, depth: usize, path: &str) -> Result<Json, JsonProblem> {
        let start = self.i;
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Json::Array(items, start..self.i));
        }
        loop {
            let child = format!("{path}[{}]", items.len());
            items.push(self.value(depth + 1, &child)?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Json::Array(items, start..self.i));
                }
                _ => return Err(self.err("',' or ']' was expected")),
            }
        }
    }

    /// A string, unescaped (for comparing names), and its span.
    fn string(&mut self) -> Result<(String, Range<usize>), JsonProblem> {
        let start = self.i;
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else {
                return Err(self.err("a string isn't closed"));
            };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok((out, start..self.i));
                }
                b'\\' => {
                    self.i += 1;
                    let Some(e) = self.peek() else {
                        return Err(self.err("a string isn't closed"));
                    };
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err(self.err("an invalid escape")),
                    };
                    out.push(ch);
                }
                0x00..=0x1f => return Err(self.err("a control character inside a string")),
                _ => {
                    let ch = self.src[self.i..]
                        .chars()
                        .next()
                        .ok_or_else(|| self.err("a string isn't closed"))?;
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    /// After `\u`: four hex digits, a surrogate pair as one character. A lone surrogate isn't
    /// a Unicode character: refused (it couldn't be compared as a name).
    fn unicode_escape(&mut self) -> Result<char, JsonProblem> {
        let hi = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&hi) {
            if self.s.get(self.i..self.i + 2) != Some(&b"\\u"[..]) {
                return Err(self.err("an escaped lone surrogate"));
            }
            self.i += 2;
            let lo = self.hex4()?;
            if !(0xDC00..0xE000).contains(&lo) {
                return Err(self.err("an escaped lone surrogate"));
            }
            0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
        } else {
            hi
        };
        char::from_u32(code).ok_or_else(|| self.err("an escaped lone surrogate"))
    }

    fn hex4(&mut self) -> Result<u32, JsonProblem> {
        let s = self.s;
        let digits = s
            .get(self.i..self.i + 4)
            .filter(|d| d.iter().all(u8::is_ascii_hexdigit))
            .ok_or_else(|| self.err("\\u needs four hex digits"))?;
        let v = digits
            .iter()
            .fold(0u32, |v, &d| v * 16 + (d as char).to_digit(16).unwrap_or(0));
        self.i += 4;
        Ok(v)
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - start
    }

    /// A number, kept as its lexeme's span (never converted here).
    fn number(&mut self) -> Result<Json, JsonProblem> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.err("a digit was expected")),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if self.digits() == 0 {
                return Err(self.err("a digit was expected after '.'"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(self.err("a digit was expected in the exponent"));
            }
        }
        Ok(Json::Number(start..self.i))
    }

    fn literal(&mut self) -> Result<Json, JsonProblem> {
        for word in [&b"true"[..], b"false", b"null"] {
            if self.s[self.i..].starts_with(word) {
                let start = self.i;
                self.i += word.len();
                return Ok(Json::Literal(start..self.i));
            }
        }
        Err(self.err("an unexpected character"))
    }
}

// ---- the version 1 values

const COLOR_FORMAT: &str = "a colour is \"#rrggbb\" or \"#rrggbbaa\"";
const UNIT: &str = "a number is taken; units such as \"16px\" aren't supported";
const SIZE: &str = "a text size is a number of points, above 0 and at most 1000";
const WEIGHT: &str = "only 400 (Regular) and 700 (Bold): the bundled Inter draws those";
const LENGTH: &str = "a length is a number from 0 to 100000";
const DIRECTION: &str = "a direction is \"horizontal\" or \"vertical\"";
const PADDING: &str = "padding is one length, or four (top, right, bottom, left)";
const KIND: &str = "this place takes another kind of value";

fn hex_color(s: &str) -> Option<Color> {
    let digits = s.strip_prefix('#')?;
    if matches!(digits.len(), 6 | 8) && digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        Color::from_hex(s)
    } else {
        None
    }
}

/// A number lexeme's value, when an f64 holds it exactly as written: finite, and its shortest
/// decimal the same number (R63). A spelling that would round (`400.00000000000000001`,
/// `-1e-99999`) gives none, so nothing rounds into a supported value or across a bound.
fn number(src: &str, r: &Range<usize>) -> Option<f64> {
    let lexeme = &src[r.clone()];
    let x = lexeme.parse::<f64>().ok().filter(|x| x.is_finite())?;
    (Decimal::of(&format!("{x}")) == Decimal::of(lexeme)).then_some(x)
}

/// A JSON number's exact value: its significant digits (no leading or trailing zero; none for
/// zero, whatever its sign) times ten to `exponent`. An exponent too large for an i64 saturates.
#[derive(Debug, PartialEq, Eq)]
struct Decimal {
    negative: bool,
    digits: String,
    exponent: i64,
}

impl Decimal {
    /// `lexeme` is a valid JSON number (or Rust's display of a finite f64).
    fn of(lexeme: &str) -> Decimal {
        let (negative, rest) = match lexeme.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, lexeme),
        };
        let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
            Some((m, e)) => (m, e),
            None => (rest, "0"),
        };
        let saturated = if exponent.starts_with('-') {
            i64::MIN / 2
        } else {
            i64::MAX / 2
        };
        let mut exponent = exponent.parse::<i64>().unwrap_or(saturated);
        let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        exponent = exponent.saturating_sub(frac.len() as i64);
        let all = format!("{int}{frac}");
        let significant = all.trim_start_matches('0');
        let digits = significant.trim_end_matches('0');
        exponent = exponent.saturating_add((significant.len() - digits.len()) as i64);
        if digits.is_empty() {
            return Decimal {
                negative: false,
                digits: String::new(),
                exponent: 0,
            };
        }
        Decimal {
            negative,
            digits: digits.to_string(),
            exponent,
        }
    }
}

fn length(v: &Json, src: &str) -> Result<f64, &'static str> {
    match v {
        Json::Number(r) => number(src, r)
            .filter(|x| (0.0..=MAX_LENGTH).contains(x))
            .ok_or(LENGTH),
        Json::String(..) => Err(UNIT),
        _ => Err(LENGTH),
    }
}

/// A value's state at its place.
fn classify(group: Group, field: Option<FrameField>, v: &Json, src: &str) -> EntryState {
    let state = match (group, field) {
        (Group::Colors, _) => match v {
            Json::String(s, _) => hex_color(s).map(TokenValue::Color).ok_or(COLOR_FORMAT),
            _ => Err(COLOR_FORMAT),
        },
        (Group::FontSizes, _) => match v {
            Json::Number(r) => number(src, r)
                .filter(|x| *x > 0.0 && *x <= MAX_FONT_SIZE)
                .map(TokenValue::Size)
                .ok_or(SIZE),
            Json::String(..) => Err(UNIT),
            _ => Err(SIZE),
        },
        (Group::FontWeights, _) => match v {
            Json::Number(r) => number(src, r)
                .and_then(|x| [400, 700].into_iter().find(|w| f64::from(*w) == x))
                .map(TokenValue::Weight)
                .ok_or(WEIGHT),
            _ => Err(WEIGHT),
        },
        (Group::Spacing | Group::Radii, _) | (Group::Frames, Some(FrameField::Gap)) => {
            length(v, src).map(TokenValue::Length)
        }
        (Group::Frames, Some(FrameField::Direction)) => match v {
            Json::String(s, _) if s == "horizontal" => Ok(TokenValue::Direction(Axis::Horizontal)),
            Json::String(s, _) if s == "vertical" => Ok(TokenValue::Direction(Axis::Vertical)),
            _ => Err(DIRECTION),
        },
        (Group::Frames, Some(FrameField::Padding)) => match v {
            Json::Number(_) => length(v, src)
                .map(|x| TokenValue::Padding([x; 4]))
                .map_err(|_| PADDING),
            Json::Array(items, _) if items.len() == 4 => {
                let mut p = [0.0; 4];
                let mut ok = true;
                for (k, item) in items.iter().enumerate() {
                    match length(item, src) {
                        Ok(x) => p[k] = x,
                        Err(_) => ok = false,
                    }
                }
                if ok {
                    Ok(TokenValue::Padding(p))
                } else {
                    Err(PADDING)
                }
            }
            _ => Err(PADDING),
        },
        (Group::Frames, None) => Err(KIND),
    };
    match state {
        Ok(v) => EntryState::Supported(v),
        Err(why) => EntryState::Unsupported(why),
    }
}

/// Whether `value` is one the place `path` takes.
fn accepts(path: &ValuePath, value: &TokenValue) -> Result<(), &'static str> {
    let len = |x: f64| x.is_finite() && (0.0..=MAX_LENGTH).contains(&x);
    let ensure = |ok: bool, why: &'static str| if ok { Ok(()) } else { Err(why) };
    match (path.group, path.field, value) {
        (Group::Colors, _, TokenValue::Color(_)) => Ok(()),
        (Group::FontSizes, _, TokenValue::Size(x)) => {
            ensure(x.is_finite() && *x > 0.0 && *x <= MAX_FONT_SIZE, SIZE)
        }
        (Group::FontWeights, _, TokenValue::Weight(w)) => ensure(matches!(w, 400 | 700), WEIGHT),
        (Group::Spacing | Group::Radii, _, TokenValue::Length(x))
        | (Group::Frames, Some(FrameField::Gap), TokenValue::Length(x)) => ensure(len(*x), LENGTH),
        (Group::Frames, Some(FrameField::Direction), TokenValue::Direction(_)) => Ok(()),
        (Group::Frames, Some(FrameField::Padding), TokenValue::Padding(p)) => {
            ensure(p.iter().all(|x| len(*x)), PADDING)
        }
        _ => Err(KIND),
    }
}

/// A number as JSON: an integer without a fraction, else the shortest round-trip decimal.
fn render_number(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

/// The JSON text of a new value (only its own span is written).
fn render(value: &TokenValue) -> String {
    match value {
        TokenValue::Color(c) => format!("\"{}\"", c.to_hex()),
        TokenValue::Size(x) | TokenValue::Length(x) => render_number(*x),
        TokenValue::Weight(w) => w.to_string(),
        TokenValue::Direction(Axis::Horizontal) => "\"horizontal\"".to_string(),
        TokenValue::Direction(Axis::Vertical) => "\"vertical\"".to_string(),
        TokenValue::Padding(p) if p.iter().all(|x| *x == p[0]) => render_number(p[0]),
        TokenValue::Padding(p) => format!(
            "[{}, {}, {}, {}]",
            render_number(p[0]),
            render_number(p[1]),
            render_number(p[2]),
            render_number(p[3])
        ),
    }
}

#[cfg(test)]
#[path = "guide_tests.rs"]
mod tests;
