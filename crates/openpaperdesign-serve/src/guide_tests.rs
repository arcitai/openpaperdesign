//! The guide codec's tests: exact bytes before and after, never parsed values alone.

use super::*;

/// The guide (with a BOM and LF, CRLF and lone-CR lines) the prose tests edit.
fn mixed() -> Vec<u8> {
    let mut g = BOM.to_vec();
    g.extend_from_slice(
        "# Guide\r\n\r\nOpening prose — Århus.\n\n## Island\rKeep\r\nthese\nbytes.\r\n\r\n\
         Ending prose — København.\r\n"
            .as_bytes(),
    );
    g
}

fn write(p: Proposal) -> Write {
    match p {
        Proposal::Write(w) => w,
        Proposal::Unchanged => panic!("a write was expected"),
    }
}

fn supported(g: &Guide) -> TokenBlock {
    match g.tokens() {
        Tokens::Supported(b) => b,
        other => panic!("supported values expected: {other:?}"),
    }
}

fn path(group: Group, name: &str) -> ValuePath {
    ValuePath {
        group,
        name: name.to_string(),
        field: None,
    }
}

fn value(b: &TokenBlock, p: &ValuePath) -> EntryState {
    b.get(p).unwrap_or_else(|| panic!("{p}")).state.clone()
}

/// Bytes `r` of `b`, as text.
fn slice(b: &[u8], r: &Range<usize>) -> String {
    String::from_utf8_lossy(&b[r.clone()]).into_owned()
}

/// `text` with `from` replaced by `to` once.
fn replaced(text: &[u8], from: &str, to: &str) -> String {
    std::str::from_utf8(text).unwrap().replacen(from, to, 1)
}

// ---- bytes

#[test]
fn reading_keeps_the_bom_and_every_terminator_exactly() {
    let bytes = mixed();
    let g = Guide::read(&bytes).unwrap();
    assert_eq!(g.bytes(), &bytes[..]);
    assert!(g.has_bom());
    let terms: Vec<Terminator> = g.lines().iter().map(|l| l.terminator).collect();
    use Terminator::*;
    assert_eq!(terms, [CrLf, CrLf, Lf, Lf, Cr, CrLf, Lf, CrLf, CrLf, CrLf]);
    assert_eq!(
        g.line_text(0),
        "# Guide",
        "the BOM isn't the first line's text"
    );
    assert_eq!(g.line_text(4), "## Island");
    // A last line without a terminator is a line.
    let g = Guide::read(b"a\rb").unwrap();
    assert_eq!(g.lines().len(), 2);
    assert_eq!(g.lines()[1].terminator, Terminator::None);
    assert_eq!(Guide::read(b"").unwrap().lines().len(), 0);
}

#[test]
fn bytes_that_are_not_a_guide_are_refused_with_where() {
    assert_eq!(
        Guide::read(&vec![b'a'; MAX_BYTES + 1]).unwrap_err(),
        ReadError::TooLarge {
            len: MAX_BYTES + 1,
            max: MAX_BYTES
        }
    );
    assert!(
        Guide::read(&vec![b'a'; MAX_BYTES]).is_ok(),
        "the limit itself is taken"
    );
    assert_eq!(
        Guide::read(b"ok\nno\0pe").unwrap_err(),
        ReadError::Nul { offset: 5 }
    );
    assert_eq!(
        Guide::read(b"ab\xffcd").unwrap_err(),
        ReadError::NotUtf8 { offset: 2 }
    );
    // A BOM-like prefix that isn't one is not UTF-8.
    assert_eq!(
        Guide::read(b"\xEF\xBBx").unwrap_err(),
        ReadError::NotUtf8 { offset: 0 }
    );
}

#[test]
fn a_revision_tells_versions_apart() {
    let a = Revision::of(b"same");
    assert_eq!(a, Revision::of(b"same"));
    assert_ne!(a, Revision::of(b"Same"));
    assert_ne!(Revision::of(b"a\n"), Revision::of(b"a\r\n"));
    assert!(a.as_str().starts_with("fnv1a64:"));
}

// ---- prose

#[test]
fn the_same_text_with_normalized_line_ends_writes_nothing() {
    let bytes = mixed();
    let g = Guide::read(&bytes).unwrap();
    let rev = g.revision().clone();
    // As an editor gives it: LF only, no BOM.
    let draft = "# Guide\n\nOpening prose — Århus.\n\n## Island\nKeep\nthese\nbytes.\n\n\
                 Ending prose — København.\n";
    assert_eq!(g.plan_text(&rev, draft).unwrap(), Proposal::Unchanged);
    // With its BOM, and as it is: still nothing.
    let with_bom = format!("\u{feff}{draft}");
    assert_eq!(g.plan_text(&rev, &with_bom).unwrap(), Proposal::Unchanged);
    let raw = std::str::from_utf8(&bytes).unwrap();
    assert_eq!(g.plan_text(&rev, raw).unwrap(), Proposal::Unchanged);
    assert_eq!(g.bytes(), &bytes[..], "planning never changes the guide");
}

#[test]
fn two_separate_prose_edits_keep_every_untouched_byte_between_them() {
    let bytes = mixed();
    let g = Guide::read(&bytes).unwrap();
    let draft = "# Guide\n\nOpening changed — Århus.\n\n## Island\nKeep\nthese\nbytes.\n\n\
                 Ending changed — København.\n";
    let w = write(g.plan_text(g.revision(), draft).unwrap());
    let mut want = BOM.to_vec();
    want.extend_from_slice(
        "# Guide\r\n\r\nOpening changed — Århus.\n\n## Island\rKeep\r\nthese\nbytes.\r\n\r\n\
         Ending changed — København.\r\n"
            .as_bytes(),
    );
    assert_eq!(
        w.bytes, want,
        "each rewritten line keeps its own terminator"
    );
    // The island between the edits (lone CR, CRLF and LF lines) is the original's, byte for
    // byte: a whole-middle replacement from a normalized draft would have made it LF.
    let island = "\n## Island\rKeep\r\nthese\nbytes.\r\n\r\n".as_bytes();
    let find = |hay: &[u8]| hay.windows(island.len()).position(|w| w == island);
    assert!(find(&bytes).is_some() && find(&w.bytes).is_some());
    // Two hunks, exactly the two lines.
    assert_eq!(w.hunks.len(), 2, "{:?}", w.hunks);
    let (h0, h1) = (&w.hunks[0], &w.hunks[1]);
    assert_eq!(slice(&bytes, &h0.original), "Opening prose — Århus.\n");
    assert_eq!(
        slice(&w.bytes, &h0.replacement),
        "Opening changed — Århus.\n"
    );
    assert_eq!(slice(&bytes, &h1.original), "Ending prose — København.\r\n");
    assert_eq!(
        slice(&w.bytes, &h1.replacement),
        "Ending changed — København.\r\n"
    );
    // Every byte outside the hunks is the original's.
    assert_eq!(bytes[..h0.original.start], w.bytes[..h0.replacement.start]);
    assert_eq!(
        bytes[h0.original.end..h1.original.start],
        w.bytes[h0.replacement.end..h1.replacement.start]
    );
    assert_eq!(bytes[h1.original.end..], w.bytes[h1.replacement.end..]);
    assert_eq!(w.base, *g.revision());
    assert_eq!(w.revision, Revision::of(&w.bytes));
}

#[test]
fn new_lines_take_the_terminator_of_the_line_before_them() {
    // Appended after an unterminated CRLF-file's last line: it gets one, and so do the new.
    let g = Guide::read(b"a\r\nb").unwrap();
    let w = write(g.plan_text(g.revision(), "a\nb\nc\n").unwrap());
    assert_eq!(w.bytes, b"a\r\nb\r\nc\r\n");
    // Inserted in a lone-CR region: lone CR.
    let g = Guide::read(b"x\ny\rz\n").unwrap();
    let w = write(g.plan_text(g.revision(), "x\ny\nnew\nz\n").unwrap());
    assert_eq!(w.bytes, b"x\ny\rnew\rz\n");
    // At the very start: the guide's first terminator. Without any: LF.
    let g = Guide::read(b"a\r\nb\n").unwrap();
    let w = write(g.plan_text(g.revision(), "top\na\nb\n").unwrap());
    assert_eq!(w.bytes, b"top\r\na\r\nb\n");
    let g = Guide::read(b"one").unwrap();
    let w = write(g.plan_text(g.revision(), "one\ntwo").unwrap());
    assert_eq!(w.bytes, b"one\ntwo");
    // Removing the final terminator is the draft's choice too; deleting lines keeps the rest.
    let g = Guide::read(b"a\r\nb\rc\n").unwrap();
    let w = write(g.plan_text(g.revision(), "a\nc").unwrap());
    assert_eq!(w.bytes, b"a\r\nc");
}

#[test]
fn a_stale_revision_plans_nothing() {
    let g = Guide::read(&mixed()).unwrap();
    let stale = Revision::of(b"an older version");
    assert_eq!(
        g.plan_text(&stale, "anything").unwrap_err(),
        PlanError::Conflict {
            expected: stale.clone(),
            actual: g.revision().clone()
        }
    );
    let p = path(Group::Colors, "accent");
    let v = TokenValue::Color(Color::rgb(1, 2, 3));
    assert!(matches!(
        g.plan_value(&stale, &p, &v),
        Err(EditError::Conflict { .. })
    ));
}

#[test]
fn a_draft_too_large_with_nul_or_too_different_is_refused_unplanned() {
    let g = Guide::read(b"a\n").unwrap();
    let rev = g.revision().clone();
    let big = "a".repeat(MAX_BYTES + 1);
    assert_eq!(
        g.plan_text(&rev, &big).unwrap_err(),
        PlanError::TooLarge {
            len: MAX_BYTES + 1,
            max: MAX_BYTES
        }
    );
    assert_eq!(
        g.plan_text(&rev, "a\0").unwrap_err(),
        PlanError::Nul { offset: 1 }
    );
    // 3000 different lines against 3000 others: beyond the budget, refused (no lossy fallback).
    let original: String = (0..3000).map(|i| format!("old {i}\n")).collect();
    let draft: String = (0..3000).map(|i| format!("new {i}\n")).collect();
    let g = Guide::read(original.as_bytes()).unwrap();
    assert!(matches!(
        g.plan_text(g.revision(), &draft),
        Err(PlanError::TooComplex { .. })
    ));
    // Long but alike (an edit near each end of 20000 lines): within it, two hunks.
    let original: String = (0..20000).map(|i| format!("line {i}\r\n")).collect();
    let draft = original
        .replace("line 7\r\n", "line seven\r\n")
        .replace("line 19990\r\n", "line end\r\n");
    let g = Guide::read(original.as_bytes()).unwrap();
    let w = write(g.plan_text(g.revision(), &draft).unwrap());
    assert_eq!(w.bytes, draft.as_bytes());
    assert_eq!(w.hunks.len(), 2);
}

#[test]
fn a_proposal_is_never_larger_than_a_guide_the_codec_reads() {
    // Run149 (the lead's probe): a MAX_BYTES guide ending in CRLF and a MAX_BYTES draft adding
    // one character after it. The CRLF is kept, so the guide it would make is one byte too
    // large: refused, nothing proposed, the guide unchanged.
    let prefix = "a".repeat(MAX_BYTES - 2);
    let original = format!("{prefix}\r\n");
    let draft = format!("{prefix}\nx");
    assert_eq!((original.len(), draft.len()), (MAX_BYTES, MAX_BYTES));
    let g = Guide::read(original.as_bytes()).unwrap();
    assert_eq!(
        g.plan_text(g.revision(), &draft),
        Err(PlanError::TooLarge {
            len: MAX_BYTES + 1,
            max: MAX_BYTES
        })
    );
    assert_eq!(g.bytes(), original.as_bytes());
    // One byte shorter it is exactly MAX_BYTES: planned, CRLF kept, and read back as planned.
    let prefix = "a".repeat(MAX_BYTES - 3);
    let g = Guide::read(format!("{prefix}\r\n").as_bytes()).unwrap();
    let w = write(g.plan_text(g.revision(), &format!("{prefix}\nx")).unwrap());
    let want = format!("{prefix}\r\nx");
    assert_eq!(w.bytes, want.as_bytes());
    assert_eq!(w.bytes.len(), MAX_BYTES);
    assert_eq!(Guide::read(&w.bytes).unwrap().bytes(), want.as_bytes());
    // The BOM counts: it is the guide's and kept, though the draft doesn't carry it.
    let with_bom = |n: usize| {
        let mut g = BOM.to_vec();
        g.extend_from_slice(format!("{}\r\n", "a".repeat(n)).as_bytes());
        g
    };
    let original = with_bom(MAX_BYTES - 5);
    assert_eq!(original.len(), MAX_BYTES);
    let g = Guide::read(&original).unwrap();
    let draft = format!("{}\nx", "a".repeat(MAX_BYTES - 5));
    assert_eq!(
        g.plan_text(g.revision(), &draft),
        Err(PlanError::TooLarge {
            len: MAX_BYTES + 1,
            max: MAX_BYTES
        })
    );
    assert_eq!(g.bytes(), &original[..]);
    let g = Guide::read(&with_bom(MAX_BYTES - 6)).unwrap();
    let draft = format!("{}\nx", "a".repeat(MAX_BYTES - 6));
    let w = write(g.plan_text(g.revision(), &draft).unwrap());
    let mut want = BOM.to_vec();
    want.extend_from_slice(format!("{}\r\nx", "a".repeat(MAX_BYTES - 6)).as_bytes());
    assert_eq!(w.bytes, want);
    assert_eq!(w.bytes.len(), MAX_BYTES);
}

// ---- values

/// A guide with a BOM, CRLF lines and the supported block, with unknown members whose number
/// lexemes and escapes no float or reserializer would keep.
fn valued() -> Vec<u8> {
    let mut g = BOM.to_vec();
    g.extend_from_slice(
        b"# Project guide\r\n\r\nOpening prose.\r\n\r\n```json\r\n{\r\n  \"opdDesignTokens\": 1,\r\n  \
          \"colors\": { \"accent\": \"#3157d5\", \"ink\": \"rgb(0, 0, 0)\" },\r\n  \
          \"fontSizes\": { \"body\": 16, \"label\": \"14px\" },\r\n  \
          \"fontWeights\": { \"regular\": 400, \"medium\": 500, \"bold\": 700 },\r\n  \
          \"spacing\": { \"s\": 8 },\r\n  \"radii\": { \"card\": 6.5 },\r\n  \
          \"frames\": { \"card\": { \"direction\": \"vertical\", \"gap\": 12, \
          \"padding\": [8, 16, 8, 16], \"wrap\": true } },\r\n  \
          \"unknown\": { \"huge\": 9007199254740993123456789, \"scale\": 1.000e+20, \
          \"escape\": \"\\u00e5\\n\" }\r\n}\r\n```\r\n## Untouched island\rKeep these bytes.\n\
          \r\nEnding prose.",
    );
    g
}

#[test]
fn the_supported_block_is_read_with_supported_and_unsupported_values() {
    let g = Guide::read(&valued()).unwrap();
    let b = supported(&g);
    assert_eq!(b.block.info, "json");
    use EntryState::*;
    assert_eq!(
        value(&b, &path(Group::Colors, "accent")),
        Supported(TokenValue::Color(Color::rgb(0x31, 0x57, 0xd5)))
    );
    assert!(matches!(
        value(&b, &path(Group::Colors, "ink")),
        Unsupported(_)
    ));
    assert_eq!(
        value(&b, &path(Group::FontSizes, "body")),
        Supported(TokenValue::Size(16.0))
    );
    assert_eq!(
        value(&b, &path(Group::FontSizes, "label")),
        Unsupported(UNIT),
        "a unit string is shown as unsupported, not guessed"
    );
    assert_eq!(
        value(&b, &path(Group::FontWeights, "regular")),
        Supported(TokenValue::Weight(400))
    );
    assert_eq!(
        value(&b, &path(Group::FontWeights, "medium")),
        Unsupported(WEIGHT)
    );
    assert_eq!(
        value(&b, &path(Group::FontWeights, "bold")),
        Supported(TokenValue::Weight(700))
    );
    assert_eq!(
        value(&b, &path(Group::Radii, "card")),
        Supported(TokenValue::Length(6.5))
    );
    let frame = |field| ValuePath {
        group: Group::Frames,
        name: "card".to_string(),
        field: Some(field),
    };
    assert_eq!(
        value(&b, &frame(FrameField::Direction)),
        Supported(TokenValue::Direction(Axis::Vertical))
    );
    assert_eq!(
        value(&b, &frame(FrameField::Padding)),
        Supported(TokenValue::Padding([8.0, 16.0, 8.0, 16.0]))
    );
    assert_eq!(b.unknown, ["frames.card.wrap", "unknown"]);
    // Each span is exactly the value's source text.
    let span = &b.get(&path(Group::Colors, "accent")).unwrap().span;
    assert_eq!(&g.bytes()[span.clone()], b"\"#3157d5\"");
}

#[test]
fn a_known_value_edit_changes_only_its_own_bytes() {
    let bytes = valued();
    let g = Guide::read(&bytes).unwrap();
    let accent = path(Group::Colors, "accent");
    let red = TokenValue::Color(Color::rgb(0xff, 0, 0));
    let w = write(g.plan_value(g.revision(), &accent, &red).unwrap());
    let want = replaced(&bytes, "\"#3157d5\"", "\"#ff0000\"");
    assert_eq!(w.bytes, want.as_bytes(), "the rest, byte for byte");
    for kept in [
        &b"9007199254740993123456789"[..],
        b"1.000e+20",
        b"\"\\u00e5\\n\"",
        b"[8, 16, 8, 16]",
        b"\"wrap\": true",
        b"## Untouched island\rKeep these bytes.\n\r\nEnding prose.",
    ] {
        assert!(
            w.bytes.windows(kept.len()).any(|x| x == kept),
            "{}",
            String::from_utf8_lossy(kept)
        );
    }
    assert_eq!(w.hunks.len(), 1);
    assert_eq!(&w.bytes[w.hunks[0].replacement.clone()], b"\"#ff0000\"");
    // It reads back as the edit.
    let after = Guide::read(&w.bytes).unwrap();
    assert_eq!(
        value(&supported(&after), &accent),
        EntryState::Supported(red)
    );
    // A size and a padding, the same way.
    let body = path(Group::FontSizes, "body");
    let size = TokenValue::Size(18.5);
    let w = write(g.plan_value(g.revision(), &body, &size).unwrap());
    let want = replaced(&bytes, "\"body\": 16,", "\"body\": 18.5,");
    assert_eq!(w.bytes, want.as_bytes());
    let padding = ValuePath {
        group: Group::Frames,
        name: "card".to_string(),
        field: Some(FrameField::Padding),
    };
    let even = TokenValue::Padding([4.0; 4]);
    let w = write(g.plan_value(g.revision(), &padding, &even).unwrap());
    let want = replaced(&bytes, "[8, 16, 8, 16]", "4");
    assert_eq!(w.bytes, want.as_bytes());
}

#[test]
fn the_same_value_writes_nothing_and_a_wrong_or_unsupported_one_changes_nothing() {
    let g = Guide::read(&valued()).unwrap();
    let rev = g.revision().clone();
    let accent = path(Group::Colors, "accent");
    let same = TokenValue::Color(Color::rgb(0x31, 0x57, 0xd5));
    assert_eq!(
        g.plan_value(&rev, &accent, &same).unwrap(),
        Proposal::Unchanged
    );
    // 16 written as 16.0: the same value, so the lexeme isn't touched.
    let body = path(Group::FontSizes, "body");
    assert_eq!(
        g.plan_value(&rev, &body, &TokenValue::Size(16.0)).unwrap(),
        Proposal::Unchanged
    );
    // An unsupported value isn't converted.
    let medium = path(Group::FontWeights, "medium");
    assert!(matches!(
        g.plan_value(&rev, &medium, &TokenValue::Weight(400)),
        Err(EditError::Unsupported { reason: WEIGHT, .. })
    ));
    // A weight the renderer doesn't draw, another kind, a missing name: refused.
    let bold = path(Group::FontWeights, "bold");
    assert_eq!(
        g.plan_value(&rev, &bold, &TokenValue::Weight(600)),
        Err(EditError::Rejected { reason: WEIGHT })
    );
    assert_eq!(
        g.plan_value(&rev, &accent, &TokenValue::Size(3.0)),
        Err(EditError::Rejected { reason: KIND })
    );
    assert_eq!(
        g.plan_value(&rev, &body, &TokenValue::Size(f64::NAN)),
        Err(EditError::Rejected { reason: SIZE })
    );
    let missing = path(Group::Colors, "brand");
    assert_eq!(
        g.plan_value(&rev, &missing, &same),
        Err(EditError::NoSuchValue(missing.clone()))
    );
    assert_eq!(g.bytes(), &valued()[..]);
}

/// A guide with `block` fenced in it, prose around.
fn fenced(block: &str) -> Vec<u8> {
    format!("Prose before.\n{block}\nProse after.\n").into_bytes()
}

#[test]
fn two_marked_blocks_are_ambiguous_and_prose_stays_editable() {
    let one = "```json\n{\"opdDesignTokens\":1,\"colors\":{\"accent\":\"#112233\"}}\n```";
    let bytes = fenced(&format!("{one}\n{one}"));
    let g = Guide::read(&bytes).unwrap();
    match g.tokens() {
        Tokens::ReadOnly { reason, blocks } => {
            assert_eq!(reason, ReadOnly::Multiple);
            assert_eq!(blocks.len(), 2);
        }
        other => panic!("{other:?}"),
    }
    let accent = path(Group::Colors, "accent");
    let v = TokenValue::Color(Color::rgb(0, 0, 0));
    assert!(matches!(
        g.plan_value(g.revision(), &accent, &v),
        Err(EditError::NotEditable { .. })
    ));
    let text = replaced(&bytes, "Prose after.", "Prose, edited.");
    let w = write(g.plan_text(g.revision(), &text).unwrap());
    assert_eq!(w.bytes, text.as_bytes(), "the blocks stay as they were");
}

#[test]
fn a_fence_inside_another_fenced_block_is_its_content_not_a_block() {
    let bytes = b"````markdown\n```json\n{\"opdDesignTokens\":1,\"colors\":{\"accent\":\"#112233\"}}\n```\n````\nExample only.\n";
    let g = Guide::read(bytes).unwrap();
    assert_eq!(
        g.tokens(),
        Tokens::None {
            other_blocks: vec![]
        },
        "an ordinary guide; nothing is created"
    );
    // A longer closing fence closes, a shorter one doesn't; tildes are fences too.
    let bytes = b"~~~json\n{\"opdDesignTokens\":1,\"spacing\":{\"s\":4}}\n~~~~\n";
    assert!(matches!(
        Guide::read(bytes).unwrap().tokens(),
        Tokens::Supported(_)
    ));
    let bytes = b"````json\n{\"opdDesignTokens\":1}\n```\n";
    assert!(matches!(
        Guide::read(bytes).unwrap().tokens(),
        Tokens::ReadOnly {
            reason: ReadOnly::Unclosed,
            ..
        }
    ));
}

#[test]
fn another_format_is_listed_and_left_alone() {
    let bytes = b"---\nproject: example\n---\n# Existing guide\nKeep this prose and its custom format.\n```yaml\ncolors:\n  accent: \"#3157d5\"\n```\n";
    let g = Guide::read(bytes).unwrap();
    assert_eq!(
        g.tokens(),
        Tokens::None {
            other_blocks: vec![FencedBlock {
                lines: 5..9,
                info: "yaml".to_string()
            }]
        }
    );
    let accent = path(Group::Colors, "accent");
    let v = TokenValue::Color(Color::rgb(0, 0, 0));
    assert!(matches!(
        g.plan_value(g.revision(), &accent, &v),
        Err(EditError::NotEditable { .. })
    ));
    // An edit of its prose keeps the front matter and the YAML exactly.
    let text = replaced(bytes, "# Existing guide", "# Guide");
    let w = write(g.plan_text(g.revision(), &text).unwrap());
    assert_eq!(w.bytes, text.as_bytes());
}

#[test]
fn invalid_json_repeated_names_depth_and_versions_make_values_read_only() {
    let read_only = |block: &str| match Guide::read(&fenced(block)).unwrap().tokens() {
        Tokens::ReadOnly { reason, .. } => reason,
        other => panic!("{block}: {other:?}"),
    };
    // A repeated name, also one spelled with an escape.
    assert_eq!(
        read_only(
            "```json\n{\"opdDesignTokens\":1,\"colors\":{\"accent\":\"#112233\",\"\\u0061ccent\":\"#ffffff\"}}\n```"
        ),
        ReadOnly::DuplicateKey {
            path: "colors".to_string(),
            key: "accent".to_string()
        }
    );
    assert_eq!(
        read_only("```json\n{\"opdDesignTokens\":1,\"opdDesignTokens\":1}\n```"),
        ReadOnly::DuplicateKey {
            path: String::new(),
            key: "opdDesignTokens".to_string()
        }
    );
    // Invalid JSON, with the guide's own byte offset: "Prose before.\n```json\n" is 22 bytes,
    // and the member name is missing after the 31-byte `{"opdDesignTokens":1,"colors":{` and
    // its line end.
    match read_only("```json\n{\"opdDesignTokens\":1,\"colors\":{\n```") {
        ReadOnly::Invalid { offset, .. } => assert_eq!(offset, 22 + 32),
        other => panic!("{other:?}"),
    }
    for bad in [
        "{\"opdDesignTokens\":1,}",
        "{\"opdDesignTokens\":01}",
        "{\"opdDesignTokens\":1,\"x\":\"\\ud800\"}",
        "{\"opdDesignTokens\":1,\"x\":\"tab\there\"}",
        "{\"opdDesignTokens\":1} trailing",
    ] {
        let reason = read_only(&format!("```json\n{bad}\n```"));
        assert!(matches!(reason, ReadOnly::Invalid { .. }), "{bad}");
    }
    let deep = format!(
        "{{\"opdDesignTokens\":1,\"x\":{}{}}}",
        "[".repeat(100),
        "]".repeat(100)
    );
    let reason = read_only(&format!("```json\n{deep}\n```"));
    assert_eq!(reason, ReadOnly::TooDeep);
    assert_eq!(
        read_only("```json\n{\"opdDesignTokens\":2}\n```"),
        ReadOnly::Version {
            lexeme: "2".to_string()
        }
    );
    assert_eq!(
        read_only("```json\n{\"opdDesignTokens\":1.0}\n```"),
        ReadOnly::Version {
            lexeme: "1.0".to_string()
        }
    );
    // Invalid JSON without the marker is just another JSON block.
    let g = Guide::read(&fenced("```json\n{not json\n```")).unwrap();
    assert!(matches!(
        g.tokens(),
        Tokens::None { other_blocks } if other_blocks.len() == 1
    ));
}

#[test]
fn a_refusal_never_changes_the_guide() {
    for block in [
        "```json\n{\"opdDesignTokens\":1,\"colors\":{\"accent\":\"#112233\",\"accent\":\"#000000\"}}\n```",
        "```json\n{\"opdDesignTokens\":1,\"colors\":{\n```",
        "```yaml\ncolors: {}\n```",
    ] {
        let bytes = fenced(block);
        let g = Guide::read(&bytes).unwrap();
        let accent = path(Group::Colors, "accent");
        let v = TokenValue::Color(Color::rgb(0, 0, 0));
        assert!(g.plan_value(g.revision(), &accent, &v).is_err(), "{block}");
        assert_eq!(g.bytes(), &bytes[..], "{block}");
    }
}
