//! The unfinished literal value of a number field (the Design panel's, and a Theme text style's
//! size), as a host hands it over to another editor of the same canvas and takes it up again
//! (issue #23, R78-03). egui's `DragValue` keeps the text being typed in its own private state
//! (`EditState`, egui 0.36.2 `widgets/drag_value.rs`), so it is seen here only through what
//! the widget's public API offers: its parser, which it calls with the raw text every time that
//! text changes (`custom_parser`, the same parse as egui's default one), and taken up the way a
//! person would: the field is focused (egui selects its text), then the raw text arrives as one
//! paste into it. Nothing is applied that typing it wouldn't apply; nothing is finished.
//!
//! A field is identified by what it shows, not by egui's per-frame ids: the shown page, the
//! layers it edits (the selection the panel shows), its label and its place among fields with
//! the same label and layers in that frame. Ephemeral UI state in egui's memory, never a value
//! store or a document change.

use crate::editor::Editor;
use crate::model::NodeId;

/// A number field, as the panel draws it (see the module docs).
#[derive(Clone, Debug, PartialEq)]
pub struct FieldTarget {
    /// The shown page's id.
    pub page: String,
    /// The layers it edits (empty for a Theme style's size).
    pub ids: Vec<NodeId>,
    /// Its label (the field's prefix, "X  ", "W  "…, or "Theme size" and the style).
    pub label: String,
    /// Its place among the fields with this label and these layers in the frame.
    pub nth: usize,
}

/// The number field that has the keyboard and the raw text typed into it, if it was typed.
#[derive(Clone, Debug)]
pub struct Focused {
    pub target: FieldTarget,
    pub text: Option<String>,
    id: egui::Id,
}

/// A value being taken up: the field to focus and the text to paste into it, in steps.
#[derive(Clone)]
struct Resume {
    target: FieldTarget,
    text: String,
    step: Step,
    /// Frames left before an unmet resume (its field isn't drawn) is dropped.
    frames: u32,
    id: Option<egui::Id>,
}

#[derive(Clone, Copy, PartialEq)]
enum Step {
    /// Waiting for the field: it is focused the frame it is drawn.
    Focus,
    /// Focused: egui selects its text on the next frame.
    Selecting,
    /// The text goes in as a paste over that selection.
    Paste,
}

fn focused_key() -> egui::Id {
    egui::Id::new("opd-field-draft-focused")
}

fn resume_key() -> egui::Id {
    egui::Id::new("opd-field-draft-resume")
}

fn seen_key() -> egui::Id {
    egui::Id::new("opd-field-draft-seen")
}

fn outcome_key() -> egui::Id {
    egui::Id::new("opd-field-draft-outcome")
}

/// At the start of each frame: the count of fields seen restarts, and an unmet resume ages.
pub fn begin_frame(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Vec<(FieldTarget, usize)>>(seen_key()));
    let gone = ctx.data_mut(|d| {
        let r = d.get_temp_mut_or_insert_with::<Option<Resume>>(resume_key(), || None);
        match r {
            Some(p) if p.frames == 0 => {
                *r = None;
                true
            }
            Some(p) => {
                p.frames -= 1;
                false
            }
            None => false,
        }
    });
    if gone {
        set_outcome(
            ctx,
            Some(
                "its field isn't shown here (the layers or the panel aren't), so it wasn't opened",
            ),
        );
    }
}

/// The parse a field's text gets (egui's default number parse: spaces ignored, U+2212 a minus).
pub fn parse_number(text: &str) -> Option<f64> {
    let t: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == '\u{2212}' { '-' } else { c })
        .collect();
    t.parse().ok()
}

/// The field this frame's next number field with `label` editing `ids` is.
pub fn target(ctx: &egui::Context, ed: &Editor, label: &str, ids: &[NodeId]) -> FieldTarget {
    let mut t = FieldTarget {
        page: ed.active_page_id().to_string(),
        ids: ids.to_vec(),
        label: label.to_string(),
        nth: 0,
    };
    ctx.data_mut(|d| {
        let seen = d.get_temp_mut_or_default::<Vec<(FieldTarget, usize)>>(seen_key());
        match seen
            .iter_mut()
            .find(|(s, _)| s.label == t.label && s.ids == t.ids)
        {
            Some((_, n)) => {
                *n += 1;
                t.nth = *n;
            }
            None => seen.push((t.clone(), 0)),
        }
    });
    t
}

/// Before the field `target` is added: a resume pasting into it now puts its text in this
/// frame's input, so the field's own text edit takes it as typed.
pub fn before(ui: &mut egui::Ui, target: &FieldTarget) {
    let ctx = ui.ctx().clone();
    let paste = ctx.data(|d| {
        d.get_temp::<Option<Resume>>(resume_key())
            .flatten()
            .filter(|r| r.target == *target && r.step == Step::Paste)
    });
    let Some(r) = paste else { return };
    if !r.id.is_some_and(|id| ctx.memory(|m| m.has_focus(id))) {
        return;
    }
    // Its shown value selected first (as a person's Command/Ctrl+A), so the text replaces it
    // instead of being appended to it.
    ui.input_mut(|i| {
        i.events.push(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        i.events.push(egui::Event::Paste(r.text.clone()));
    });
    ctx.data_mut(|d| d.insert_temp::<Option<Resume>>(resume_key(), None));
    set_outcome(&ctx, None);
}

/// After the field `target` was added (`r` its response; `typed` the raw text its parser saw
/// this frame, if any): keep what it holds while it has the keyboard, and move a resume of it
/// on (focus it, then let egui select its text).
pub fn after(ctx: &egui::Context, r: &egui::Response, target: FieldTarget, typed: Option<String>) {
    let resume = ctx.data(|d| d.get_temp::<Option<Resume>>(resume_key()).flatten());
    if let Some(mut p) = resume.filter(|p| p.target == target) {
        match p.step {
            // Only once it can take the focus (not in a popup's first, disabled sizing pass).
            Step::Focus if !r.enabled() => {}
            Step::Focus => {
                ctx.memory_mut(|m| m.request_focus(r.id));
                p.step = Step::Selecting;
                p.id = Some(r.id);
            }
            Step::Selecting => p.step = Step::Paste,
            Step::Paste => {}
        }
        ctx.data_mut(|d| d.insert_temp::<Option<Resume>>(resume_key(), Some(p)));
        ctx.request_repaint();
    }
    let held = ctx.data(|d| d.get_temp::<Option<Focused>>(focused_key()).flatten());
    if r.has_focus() {
        // The text typed so far: this frame's, else what it had (typing changes it; a frame
        // without typing keeps it).
        let text = typed.or_else(|| held.filter(|f| f.id == r.id).and_then(|f| f.text));
        let f = Focused {
            target,
            text,
            id: r.id,
        };
        ctx.data_mut(|d| d.insert_temp::<Option<Focused>>(focused_key(), Some(f)));
    } else if held.is_some_and(|f| f.id == r.id) {
        ctx.data_mut(|d| d.insert_temp::<Option<Focused>>(focused_key(), None));
    }
}

/// The number field that has the keyboard now and the raw text typed into it.
pub fn focused(ctx: &egui::Context) -> Option<Focused> {
    let f = ctx.data(|d| d.get_temp::<Option<Focused>>(focused_key()).flatten())?;
    ctx.memory(|m| m.has_focus(f.id)).then_some(f)
}

/// Take up `text` in the number field `target` over the next frames (see the module docs).
/// The caller has shown its layers and panel.
pub fn resume(ctx: &egui::Context, target: FieldTarget, text: String) {
    let r = Resume {
        target,
        text,
        step: Step::Focus,
        frames: 30,
        id: None,
    };
    ctx.data_mut(|d| d.insert_temp::<Option<Resume>>(resume_key(), Some(r)));
    set_outcome(ctx, Some("still opening"));
    ctx.request_repaint();
}

/// What became of the last resume: `None` taken up, else why not (or "still opening").
pub fn outcome(ctx: &egui::Context) -> Option<String> {
    ctx.data(|d| d.get_temp::<Option<String>>(outcome_key()).flatten())
}

fn set_outcome(ctx: &egui::Context, why: Option<&str>) {
    let v = why.map(str::to_string);
    ctx.data_mut(|d| d.insert_temp::<Option<String>>(outcome_key(), v));
}
