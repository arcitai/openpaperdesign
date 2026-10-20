//! macOS-flavoured look for the egui chrome: Inter font, system accent, light/dark palettes
//! that follow the system appearance, and thin SF Symbols-style line icons.

use egui::{
    Color32, CornerRadius, FontData, FontFamily, FontId, Pos2, Rect, Shadow, Stroke, Vec2, pos2,
    vec2,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

pub static INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
pub static INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
pub static INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

/// macOS system blue, used when the system accent can't be read.
pub const MAC_BLUE: Color32 = Color32::from_rgb(0x00, 0x7a, 0xff);

/// The chrome's type scale (DESIGN.md, revision 19): compact 12pt body text in the panels,
/// 11pt secondary text, and 13pt for the one title a panel has (the Canvas's name). Section
/// titles are the body size in Inter Medium, so the hierarchy comes from weight and colour,
/// not from size jumps.
pub const BODY: f32 = 12.0;
pub const SECONDARY: f32 = 11.0;
pub const TITLE: f32 = 13.0;
/// Height of the unified toolbar.
pub const TOOLBAR_H: f32 = 40.0;
/// Traffic lights: left inset and the height of the bar they are centred in (points).
pub const TRAFFIC_LIGHTS: (f64, f64) = (14.0, TOOLBAR_H as f64);
/// Horizontal room the traffic lights need from the window's left edge.
pub const TRAFFIC_LIGHTS_W: f32 = 76.0;
/// The corner of controls, fields, rows and toggles in the chrome (floating surfaces use 10).
pub const RADIUS: f32 = 5.0;
/// The height of an inspector field (a number, a choice, a text): egui's interact height, so
/// paired fields, their labels and the icon toggles beside them share one line.
pub const FIELD_H: f32 = 24.0;
/// The side of a chrome icon: the icons' 16 pt design grid, on every button from 24 pt up.
pub const ICON: f32 = 16.0;

static ACCENT: AtomicU32 = AtomicU32::new(u32::from_be_bytes([0x00, 0x7a, 0xff, 0xff]));

pub fn accent() -> Color32 {
    let [r, g, b, _] = ACCENT.load(Ordering::Relaxed).to_be_bytes();
    Color32::from_rgb(r, g, b)
}

pub fn set_accent(c: Color32) {
    ACCENT.store(
        u32::from_be_bytes([c.r(), c.g(), c.b(), 0xff]),
        Ordering::Relaxed,
    );
}

/// The user's accent colour from System Settings › Appearance (falls back to system blue).
pub fn system_accent() -> Color32 {
    #[cfg(target_os = "macos")]
    {
        // `AppleAccentColor` is absent for the default (blue) and multicolour settings.
        let out = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleAccentColor"])
            .output();
        if let Ok(o) = out
            && o.status.success()
        {
            return match String::from_utf8_lossy(&o.stdout).trim() {
                "-1" => Color32::from_rgb(0x8c, 0x8c, 0x8c),
                "0" => Color32::from_rgb(0xff, 0x3b, 0x30),
                "1" => Color32::from_rgb(0xff, 0x95, 0x00),
                "2" => Color32::from_rgb(0xff, 0xcc, 0x00),
                "3" => Color32::from_rgb(0x28, 0xcd, 0x41),
                "5" => Color32::from_rgb(0xaf, 0x52, 0xde),
                "6" => Color32::from_rgb(0xff, 0x2d, 0x55),
                _ => MAC_BLUE,
            };
        }
    }
    MAC_BLUE
}

/// Colours for one appearance, loosely after AppKit's semantic colours.
#[derive(Clone, Copy)]
pub struct Palette {
    pub window: Color32,
    pub sidebar: Color32,
    pub inspector: Color32,
    pub control: Color32,
    pub control_border: Color32,
    pub field: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub tertiary: Color32,
    pub separator: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub popover: Color32,
}

/// The chrome is one calm, neutral surface around the canvas (see DESIGN.md, revision 19):
/// both full-height side panels share one panel colour, separated from the canvas by one
/// vertical hairline each; fields and buttons are filled a step darker than the panel with a
/// faint edge rather than outlined white boxes; greys are neutral (no blue cast), so the accent
/// is left for the current tool, the selection and keyboard focus. Secondary text keeps at
/// least 4.5:1 on the panel at 11pt; tertiary is for disabled controls only.
pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            window: Color32::from_rgb(0x1e, 0x1e, 0x1f),
            sidebar: Color32::from_rgb(0x1e, 0x1e, 0x1f),
            inspector: Color32::from_rgb(0x1e, 0x1e, 0x1f),
            control: Color32::from_rgb(0x2c, 0x2c, 0x2e),
            control_border: Color32::from_white_alpha(14),
            field: Color32::from_rgb(0x2a, 0x2a, 0x2c),
            text: Color32::from_rgb(0xec, 0xec, 0xed),
            secondary: Color32::from_rgb(0xa2, 0xa2, 0xa6),
            tertiary: Color32::from_rgb(0x62, 0x62, 0x66),
            separator: Color32::from_rgb(0x30, 0x30, 0x33),
            hover: Color32::from_white_alpha(12),
            pressed: Color32::from_white_alpha(20),
            popover: Color32::from_rgb(0x26, 0x26, 0x28),
        }
    } else {
        Palette {
            window: Color32::from_rgb(0xfa, 0xfa, 0xfa),
            sidebar: Color32::from_rgb(0xfa, 0xfa, 0xfa),
            inspector: Color32::from_rgb(0xfa, 0xfa, 0xfa),
            control: Color32::WHITE,
            control_border: Color32::from_black_alpha(14),
            field: Color32::from_rgb(0xf0, 0xf0, 0xf1),
            text: Color32::from_rgb(0x1c, 0x1c, 0x1e),
            secondary: Color32::from_rgb(0x6c, 0x6c, 0x70),
            tertiary: Color32::from_rgb(0xae, 0xae, 0xb2),
            separator: Color32::from_rgb(0xe8, 0xe8, 0xea),
            hover: Color32::from_black_alpha(8),
            pressed: Color32::from_black_alpha(14),
            popover: Color32::WHITE,
        }
    }
}

/// The accent as text, a glyph or a 1 px line on its own tint (`selected_fill`, egui's
/// selection): a step darker in light, lighter in dark, so a chosen menu item, the current
/// tool's glyph or a focused field's edge stays readable.
pub fn accent_ink(dark: bool) -> Color32 {
    let a = accent();
    if dark {
        blend(a, Color32::from_white_alpha(90))
    } else {
        // About 4.6:1 for 12 pt text on the light tint (the plain accent is under 4:1 there).
        blend(a, Color32::from_black_alpha(60))
    }
}

/// egui's selection: a translucent accent tint (text selections, chosen menu items, sliders),
/// never a solid accent block with white text.
fn selection_fill(dark: bool) -> Color32 {
    let a = accent();
    Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), if dark { 90 } else { 52 })
}

/// The fill of a selected row, tool or toggle: a tint of the accent, so the label stays in the
/// ordinary text colour and the selection reads without shouting.
pub fn selected_fill(ui: &egui::Ui) -> Color32 {
    let base = pal(ui).sidebar;
    let a = accent();
    let alpha = if ui.visuals().dark_mode { 72 } else { 38 };
    blend(
        base,
        Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), alpha),
    )
}

pub fn pal(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

/// Inter (Regular/Medium/SemiBold) as the proportional family, egui's fonts as fallbacks.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("Inter", INTER_REGULAR),
        ("Inter-Medium", INTER_MEDIUM),
        ("Inter-SemiBold", INTER_SEMIBOLD),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let with = |first: &str| {
        let mut v = vec![first.to_string()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with("Inter"));
    fonts
        .families
        .insert(FontFamily::Name("medium".into()), with("Inter-Medium"));
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), with("Inter-SemiBold"));
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        mono.push("Inter".into());
    }
    ctx.set_fonts(fonts);
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("medium".into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

fn style_for(s: &mut egui::Style, dark: bool) {
    use egui::TextStyle as T;
    let p = palette(dark);
    let acc = accent();
    s.visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    s.text_styles = [
        (T::Small, FontId::proportional(SECONDARY)),
        (T::Body, FontId::proportional(BODY)),
        (T::Button, FontId::proportional(BODY)),
        (T::Heading, semibold(TITLE)),
        (T::Monospace, FontId::monospace(SECONDARY)),
    ]
    .into();
    s.spacing.item_spacing = vec2(6.0, 4.0);
    s.spacing.button_padding = vec2(8.0, 3.0);
    s.spacing.interact_size = vec2(24.0, FIELD_H);
    s.spacing.menu_margin = egui::Margin::same(5);
    s.spacing.window_margin = egui::Margin::same(12);
    s.spacing.icon_width = 13.0;
    s.spacing.icon_width_inner = 7.0;
    s.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 7.0,
        handle_min_length: 24.0,
        ..egui::style::ScrollStyle::floating()
    };
    s.interaction.selectable_labels = false;

    let v = &mut s.visuals;
    v.override_text_color = None;
    v.weak_text_color = Some(p.secondary);
    v.hyperlink_color = acc;
    v.panel_fill = p.inspector;
    v.window_fill = p.popover;
    v.faint_bg_color = if dark {
        Color32::from_white_alpha(6)
    } else {
        Color32::from_black_alpha(6)
    };
    v.extreme_bg_color = p.field;
    v.text_edit_bg_color = Some(p.field);
    v.window_stroke = Stroke::new(1.0, p.control_border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    // Floating surfaces (the rail, zoom, menus, the floating top rows) lift with one soft,
    // close shadow and a hairline edge, not a heavy drop.
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 100 } else { 30 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 2],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 80 } else { 20 }),
    };
    // A tint, with the accent's ink as its text and as a focused field's 1 px edge.
    v.selection.bg_fill = selection_fill(dark);
    v.selection.stroke = Stroke::new(1.0, accent_ink(dark));
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.indent_has_left_vline = false;
    v.striped = false;
    v.interact_cursor = None;
    v.collapsing_header_frame = false;

    // One corner for controls, fields, rows and toggles alike (`RADIUS`). Buttons, number
    // fields, choices and text fields are one family: filled `field`, a faint edge that keeps
    // checkboxes and radios legible, a darker fill on hover and press. egui draws a focused
    // widget with the `active` look, so its accent-ink edge is the keyboard focus (and an open
    // choice's); a focused text field gets the same edge from `selection.stroke`.
    let r = CornerRadius::same(RADIUS as u8);
    let edge = Stroke::new(1.0, p.control_border);
    let field_hover = blend(p.field, p.hover);
    let field_pressed = blend(p.field, p.pressed);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.inspector;
    w.noninteractive.weak_bg_fill = p.inspector;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = p.field;
    w.inactive.weak_bg_fill = p.field;
    w.inactive.bg_stroke = edge;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;
    w.hovered.bg_fill = field_hover;
    w.hovered.weak_bg_fill = field_hover;
    w.hovered.bg_stroke = edge;
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = field_pressed;
    w.active.weak_bg_fill = field_pressed;
    w.active.bg_stroke = Stroke::new(1.0, accent_ink(dark));
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open = w.active;
}

/// The glyph's place in an icon button `rect`: `ICON` square on the ordinary 24 pt and larger
/// buttons, 4 pt clear of the edges on smaller ones; centred, so icons line up across rows.
pub fn icon_rect(rect: Rect) -> Rect {
    let side = (rect.width().min(rect.height()) - 8.0).clamp(8.0, ICON);
    Rect::from_center_size(rect.center(), Vec2::splat(side))
}

/// A disclosure chevron for a folded section (`egui::CollapsingHeader::icon`): the same quiet
/// line chevron as the Papers list and layer rows, not egui's filled triangle.
pub fn disclosure(ui: &mut egui::Ui, openness: f32, r: &egui::Response) {
    let icon = if openness > 0.5 {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let color = if r.hovered() {
        pal(ui).text
    } else {
        pal(ui).secondary
    };
    let glyph = Rect::from_center_size(r.rect.center(), Vec2::splat(10.0));
    paint_icon(ui.painter(), glyph, icon, color);
}

/// `over` composited on `base`.
pub fn blend(base: Color32, over: Color32) -> Color32 {
    let a = over.a() as f32 / 255.0;
    let [or, og, ob, _] = over.to_srgba_unmultiplied();
    let mix = |b: u8, o: u8| (b as f32 * (1.0 - a) + o as f32 * a).round() as u8;
    Color32::from_rgb(mix(base.r(), or), mix(base.g(), og), mix(base.b(), ob))
}

/// Fonts plus light and dark styles. egui picks the one matching the system appearance
/// (reported by egui-winit); headless rendering falls back to light.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.options_mut(|o| {
        o.theme_preference = egui::ThemePreference::System;
        o.fallback_theme = egui::Theme::Light;
    });
    ctx.style_mut_of(egui::Theme::Light, |s| style_for(s, false));
    ctx.style_mut_of(egui::Theme::Dark, |s| style_for(s, true));
}

/// Thin line icons in the spirit of SF Symbols, drawn into a square `rect`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Pointer,
    Hand,
    Frame,
    Rect,
    Ellipse,
    Line,
    Text,
    Pencil,
    Eraser,
    Ink,
    Eye,
    EyeSlash,
    Lock,
    LockOpen,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    Question,
    Plus,
    Minus,
    Sidebar,
    /// [`Icon::Sidebar`] mirrored: the right panel's toggle.
    SidebarRight,
    Export,
    Grid,
    Trash,
    More,
    Page,
    Restore,
    Group,
    Pen,
    Polygon,
    Star,
    Component,
    Instance,
    Image,
    AlignLeft,
    AlignHCenter,
    AlignRight,
    AlignTop,
    AlignVCenter,
    AlignBottom,
    DistributeH,
    DistributeV,
    Corners,
    Wrap,
    ArrowRight,
    ArrowDown,
    SpaceBetween,
    Arrow,
    Sticky,
    Diamond,
    Laser,
    Library,
    Sketch,
    Undo,
    Redo,
}

pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 16.0; // design grid: 16x16
    let pt = |x: f32, y: f32| -> Pos2 { pos2(c.x + (x - 8.0) * u, c.y + (y - 8.0) * u) };
    // One line weight across the chrome: 1.2 pt at the 16 pt grid, never under 1 or over 1.4.
    let w = (1.2 * u).clamp(1.0, 1.4);
    let st = Stroke::new(w, color);
    let line = |pts: &[Pos2]| painter.add(egui::Shape::line(pts.to_vec(), st));
    let closed = |pts: &[Pos2]| painter.add(egui::Shape::closed_line(pts.to_vec(), st));
    match icon {
        Icon::Pointer => {
            closed(&[
                pt(4.5, 2.5),
                pt(12.5, 9.5),
                pt(8.8, 9.9),
                pt(10.9, 14.0),
                pt(9.4, 14.7),
                pt(7.3, 10.6),
                pt(4.5, 13.0),
            ]);
        }
        Icon::Hand => {
            for (x, top) in [(5.5, 5.0), (7.7, 3.0), (9.9, 3.5), (12.0, 5.5)] {
                line(&[pt(x, 9.0), pt(x, top)]);
                painter.circle_stroke(pt(x, top), 0.01, st);
            }
            line(&[pt(5.5, 9.0), pt(5.5, 11.0), pt(3.3, 9.0)]);
            line(&[pt(12.0, 5.5), pt(12.0, 11.0)]);
            painter.add(egui::Shape::line(
                arc(pt(8.75, 11.0), 3.25 * u, 0.0, std::f32::consts::PI, 10),
                st,
            ));
        }
        Icon::Frame => {
            line(&[pt(5.0, 2.0), pt(5.0, 14.0)]);
            line(&[pt(11.0, 2.0), pt(11.0, 14.0)]);
            line(&[pt(2.0, 5.0), pt(14.0, 5.0)]);
            line(&[pt(2.0, 11.0), pt(14.0, 11.0)]);
        }
        Icon::Rect => {
            painter.rect_stroke(
                Rect::from_min_max(pt(2.5, 3.5), pt(13.5, 12.5)),
                2.0 * u,
                st,
                egui::StrokeKind::Middle,
            );
        }
        Icon::Ellipse => {
            painter.circle_stroke(c, 5.75 * u, st);
        }
        Icon::Line => {
            line(&[pt(3.0, 13.0), pt(13.0, 3.0)]);
        }
        Icon::Text => {
            line(&[pt(3.5, 4.0), pt(3.5, 3.0), pt(12.5, 3.0), pt(12.5, 4.0)]);
            line(&[pt(8.0, 3.0), pt(8.0, 13.0)]);
            line(&[pt(6.3, 13.0), pt(9.7, 13.0)]);
        }
        Icon::Pencil => {
            closed(&[
                pt(11.0, 2.2),
                pt(13.8, 5.0),
                pt(5.5, 13.3),
                pt(2.2, 13.8),
                pt(2.7, 10.5),
            ]);
            line(&[pt(9.3, 3.9), pt(12.1, 6.7)]);
        }
        Icon::Eraser => {
            closed(&[
                pt(9.5, 2.5),
                pt(14.0, 7.0),
                pt(8.0, 13.0),
                pt(5.0, 13.0),
                pt(2.0, 10.0),
            ]);
            line(&[pt(5.8, 6.2), pt(10.3, 10.7)]);
            line(&[pt(8.0, 13.0), pt(14.0, 13.0)]);
        }
        Icon::Ink => {
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    pt(
                        2.5 + 11.0 * t,
                        8.0 - 3.2 * (t * std::f32::consts::TAU).sin(),
                    )
                })
                .collect();
            line(&pts);
        }
        Icon::Eye | Icon::EyeSlash => {
            let mut top = arc(pt(8.0, 13.5), 7.5 * u, -2.4, -0.74, 10);
            let bottom = arc(pt(8.0, 2.5), 7.5 * u, 0.74, 2.4, 10);
            top.extend(bottom);
            closed(&top);
            painter.circle_stroke(c, 2.0 * u, st);
            if icon == Icon::EyeSlash {
                line(&[pt(3.0, 3.0), pt(13.0, 13.0)]);
            }
        }
        Icon::Lock | Icon::LockOpen => {
            painter.rect_stroke(
                Rect::from_min_max(pt(3.5, 7.5), pt(12.5, 14.0)),
                1.5 * u,
                st,
                egui::StrokeKind::Middle,
            );
            let (cx, a0) = if icon == Icon::Lock {
                (8.0, 0.0)
            } else {
                (10.5, 0.0)
            };
            let mut sh = vec![pt(cx - 2.5, 7.5)];
            sh.extend(arc(
                pt(cx, 5.0),
                2.5 * u,
                std::f32::consts::PI,
                std::f32::consts::TAU + a0,
                10,
            ));
            if icon == Icon::Lock {
                sh.push(pt(cx + 2.5, 7.5));
            }
            line(&sh);
        }
        Icon::ChevronRight => {
            line(&[pt(6.0, 3.5), pt(10.5, 8.0), pt(6.0, 12.5)]);
        }
        Icon::ChevronDown => {
            line(&[pt(3.5, 6.0), pt(8.0, 10.5), pt(12.5, 6.0)]);
        }
        Icon::ChevronUp => {
            line(&[pt(3.5, 10.0), pt(8.0, 5.5), pt(12.5, 10.0)]);
        }
        Icon::Question => {
            painter.circle_stroke(c, 6.5 * u, st);
            painter.text(
                c + vec2(0.0, 0.3 * u),
                egui::Align2::CENTER_CENTER,
                "?",
                medium(9.5 * u),
                color,
            );
        }
        Icon::Plus => {
            line(&[pt(8.0, 3.0), pt(8.0, 13.0)]);
            line(&[pt(3.0, 8.0), pt(13.0, 8.0)]);
        }
        Icon::Minus => {
            line(&[pt(3.0, 8.0), pt(13.0, 8.0)]);
        }
        Icon::Sidebar | Icon::SidebarRight => {
            painter.rect_stroke(
                Rect::from_min_max(pt(1.5, 3.0), pt(14.5, 13.0)),
                2.0 * u,
                st,
                egui::StrokeKind::Middle,
            );
            let x = if icon == Icon::Sidebar { 6.0 } else { 10.0 };
            line(&[pt(x, 3.0), pt(x, 13.0)]);
        }
        Icon::Group => {
            let dash = |a: Pos2, b: Pos2| {
                painter.add(egui::Shape::dashed_line(&[a, b], st, 1.6 * u, 1.4 * u));
            };
            dash(pt(2.5, 2.5), pt(13.5, 2.5));
            dash(pt(13.5, 2.5), pt(13.5, 13.5));
            dash(pt(13.5, 13.5), pt(2.5, 13.5));
            dash(pt(2.5, 13.5), pt(2.5, 2.5));
            painter.rect_stroke(
                Rect::from_min_max(pt(5.5, 5.5), pt(10.5, 10.5)),
                1.0 * u,
                st,
                egui::StrokeKind::Middle,
            );
        }
        Icon::Pen => {
            closed(&[
                pt(8.0, 2.0),
                pt(12.5, 8.5),
                pt(10.0, 13.5),
                pt(6.0, 13.5),
                pt(3.5, 8.5),
            ]);
            line(&[pt(8.0, 2.0), pt(8.0, 8.0)]);
            painter.circle_stroke(pt(8.0, 9.0), 1.0 * u, st);
        }
        Icon::Polygon => {
            closed(&[pt(8.0, 2.0), pt(14.0, 13.5), pt(2.0, 13.5)]);
        }
        Icon::Star => {
            let pts: Vec<Pos2> = (0..10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    let r = if i % 2 == 0 { 6.5 } else { 2.8 };
                    pt(8.0 + a.cos() * r, 8.5 + a.sin() * r)
                })
                .collect();
            closed(&pts);
        }
        Icon::Component | Icon::Instance => {
            let d = [
                [pt(8.0, 1.8), pt(10.2, 4.0), pt(8.0, 6.2), pt(5.8, 4.0)],
                [pt(8.0, 9.8), pt(10.2, 12.0), pt(8.0, 14.2), pt(5.8, 12.0)],
                [pt(4.0, 5.8), pt(6.2, 8.0), pt(4.0, 10.2), pt(1.8, 8.0)],
                [pt(12.0, 5.8), pt(14.2, 8.0), pt(12.0, 10.2), pt(9.8, 8.0)],
            ];
            if icon == Icon::Component {
                for q in d {
                    closed(&q);
                }
            } else {
                closed(&[pt(8.0, 1.8), pt(14.2, 8.0), pt(8.0, 14.2), pt(1.8, 8.0)]);
            }
        }
        Icon::Image => {
            painter.rect_stroke(
                Rect::from_min_max(pt(2.0, 3.0), pt(14.0, 13.0)),
                1.5 * u,
                st,
                egui::StrokeKind::Middle,
            );
            line(&[
                pt(2.5, 12.0),
                pt(6.5, 7.5),
                pt(9.5, 10.5),
                pt(11.0, 9.0),
                pt(13.5, 11.5),
            ]);
            painter.circle_stroke(pt(10.5, 5.8), 1.1 * u, st);
        }
        Icon::AlignLeft | Icon::AlignHCenter | Icon::AlignRight => {
            let x = match icon {
                Icon::AlignLeft => 2.5,
                Icon::AlignHCenter => 8.0,
                _ => 13.5,
            };
            line(&[pt(x, 1.5), pt(x, 14.5)]);
            let bar = |y0: f32, w: f32| {
                let (x0, x1) = match icon {
                    Icon::AlignLeft => (4.0, 4.0 + w),
                    Icon::AlignHCenter => (8.0 - w / 2.0, 8.0 + w / 2.0),
                    _ => (12.0 - w, 12.0),
                };
                painter.rect_filled(
                    Rect::from_min_max(pt(x0, y0), pt(x1, y0 + 3.0)),
                    0.8 * u,
                    color,
                );
            };
            bar(4.0, 8.0);
            bar(9.0, 5.0);
        }
        Icon::AlignTop | Icon::AlignVCenter | Icon::AlignBottom => {
            let y = match icon {
                Icon::AlignTop => 2.5,
                Icon::AlignVCenter => 8.0,
                _ => 13.5,
            };
            line(&[pt(1.5, y), pt(14.5, y)]);
            let bar = |x0: f32, h: f32| {
                let (y0, y1) = match icon {
                    Icon::AlignTop => (4.0, 4.0 + h),
                    Icon::AlignVCenter => (8.0 - h / 2.0, 8.0 + h / 2.0),
                    _ => (12.0 - h, 12.0),
                };
                painter.rect_filled(
                    Rect::from_min_max(pt(x0, y0), pt(x0 + 3.0, y1)),
                    0.8 * u,
                    color,
                );
            };
            bar(4.0, 8.0);
            bar(9.0, 5.0);
        }
        Icon::DistributeH => {
            line(&[pt(2.0, 2.0), pt(2.0, 14.0)]);
            line(&[pt(14.0, 2.0), pt(14.0, 14.0)]);
            painter.rect_filled(
                Rect::from_min_max(pt(6.5, 4.5), pt(9.5, 11.5)),
                0.8 * u,
                color,
            );
        }
        Icon::DistributeV => {
            line(&[pt(2.0, 2.0), pt(14.0, 2.0)]);
            line(&[pt(2.0, 14.0), pt(14.0, 14.0)]);
            painter.rect_filled(
                Rect::from_min_max(pt(4.5, 6.5), pt(11.5, 9.5)),
                0.8 * u,
                color,
            );
        }
        Icon::Corners => {
            for (a, b, c2) in [
                (pt(2.5, 6.0), pt(2.5, 2.5), pt(6.0, 2.5)),
                (pt(10.0, 2.5), pt(13.5, 2.5), pt(13.5, 6.0)),
                (pt(13.5, 10.0), pt(13.5, 13.5), pt(10.0, 13.5)),
                (pt(6.0, 13.5), pt(2.5, 13.5), pt(2.5, 10.0)),
            ] {
                line(&[a, b, c2]);
            }
        }
        Icon::Wrap => {
            line(&[pt(2.0, 4.0), pt(12.0, 4.0)]);
            let mut a = arc(
                pt(12.0, 7.25),
                3.25 * u,
                -std::f32::consts::FRAC_PI_2,
                std::f32::consts::FRAC_PI_2,
                8,
            );
            a.push(pt(5.0, 10.5));
            line(&a);
            line(&[pt(7.0, 8.5), pt(5.0, 10.5), pt(7.0, 12.5)]);
        }
        Icon::ArrowRight => {
            line(&[pt(2.5, 8.0), pt(13.0, 8.0)]);
            line(&[pt(9.5, 4.5), pt(13.0, 8.0), pt(9.5, 11.5)]);
        }
        Icon::ArrowDown => {
            line(&[pt(8.0, 2.5), pt(8.0, 13.0)]);
            line(&[pt(4.5, 9.5), pt(8.0, 13.0), pt(11.5, 9.5)]);
        }
        Icon::SpaceBetween => {
            line(&[pt(1.5, 3.0), pt(1.5, 13.0)]);
            line(&[pt(14.5, 3.0), pt(14.5, 13.0)]);
            line(&[pt(4.0, 8.0), pt(12.0, 8.0)]);
            line(&[pt(6.0, 6.0), pt(4.0, 8.0), pt(6.0, 10.0)]);
            line(&[pt(10.0, 6.0), pt(12.0, 8.0), pt(10.0, 10.0)]);
        }
        Icon::Arrow => {
            line(&[pt(3.0, 13.0), pt(13.0, 3.0)]);
            line(&[pt(7.5, 3.0), pt(13.0, 3.0), pt(13.0, 8.5)]);
        }
        Icon::Sticky => {
            closed(&[
                pt(2.5, 2.5),
                pt(13.5, 2.5),
                pt(13.5, 9.5),
                pt(9.5, 13.5),
                pt(2.5, 13.5),
            ]);
            line(&[pt(13.5, 9.5), pt(9.5, 9.5), pt(9.5, 13.5)]);
        }
        Icon::Diamond => {
            closed(&[pt(8.0, 1.8), pt(14.2, 8.0), pt(8.0, 14.2), pt(1.8, 8.0)]);
        }
        Icon::Laser => {
            line(&[pt(2.5, 13.5), pt(9.5, 6.5)]);
            painter.circle_filled(pt(11.0, 5.0), 2.2 * u, color);
            for (dx, dy) in [(0.0, -3.8), (3.8, 0.0), (2.7, -2.7)] {
                line(&[
                    pt(11.0 + dx * 0.9, 5.0 + dy * 0.9),
                    pt(11.0 + dx * 1.35, 5.0 + dy * 1.35),
                ]);
            }
        }
        Icon::Library => {
            for x in [3.0, 6.5] {
                painter.rect_stroke(
                    Rect::from_min_max(pt(x, 2.5), pt(x + 2.5, 13.5)),
                    0.6 * u,
                    st,
                    egui::StrokeKind::Middle,
                );
            }
            closed(&[pt(10.0, 3.5), pt(12.3, 2.9), pt(14.6, 12.9), pt(12.3, 13.5)]);
        }
        Icon::Sketch => {
            let pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    pt(2.5 + 11.0 * t, 11.0 - 6.0 * t + 1.2 * (t * 19.0).sin())
                })
                .collect();
            line(&pts);
            line(&[pt(2.5, 13.5), pt(13.5, 13.5)]);
        }
        Icon::Undo | Icon::Redo => {
            // A curved arrow: over the top from one side, down to an arrowhead on the other.
            let m = |x: f32| if icon == Icon::Undo { x } else { 16.0 - x };
            let (a0, a1) = if icon == Icon::Undo {
                (0.0, -std::f32::consts::PI)
            } else {
                (-std::f32::consts::PI, 0.0)
            };
            let mut a = arc(pt(8.0, 9.5), 5.0 * u, a0, a1, 12);
            a.insert(0, pt(m(13.0), 13.0));
            line(&a);
            line(&[pt(m(1.0), 7.0), pt(m(3.0), 9.8), pt(m(5.6), 7.4)]);
        }
        Icon::Grid => {
            for (x, y) in [(2.5, 2.5), (9.0, 2.5), (2.5, 9.0), (9.0, 9.0)] {
                painter.rect_stroke(
                    Rect::from_min_max(pt(x, y), pt(x + 4.5, y + 4.5)),
                    1.0 * u,
                    st,
                    egui::StrokeKind::Middle,
                );
            }
        }
        Icon::Trash => {
            line(&[pt(2.5, 4.0), pt(13.5, 4.0)]);
            line(&[pt(6.0, 4.0), pt(6.0, 2.3), pt(10.0, 2.3), pt(10.0, 4.0)]);
            closed(&[pt(3.8, 4.0), pt(12.2, 4.0), pt(11.4, 14.0), pt(4.6, 14.0)]);
            line(&[pt(6.6, 6.5), pt(6.9, 11.8)]);
            line(&[pt(9.4, 6.5), pt(9.1, 11.8)]);
        }
        Icon::More => {
            for x in [3.5, 8.0, 12.5] {
                painter.circle_filled(pt(x, 8.0), 1.1 * u, color);
            }
        }
        Icon::Page => {
            closed(&[
                pt(3.5, 1.8),
                pt(9.5, 1.8),
                pt(12.5, 4.8),
                pt(12.5, 14.2),
                pt(3.5, 14.2),
            ]);
            line(&[pt(9.5, 1.8), pt(9.5, 4.8), pt(12.5, 4.8)]);
        }
        Icon::Restore => {
            let mut a = arc(c, 5.5 * u, -2.6, 2.2, 14);
            a.reverse();
            line(&a);
            let start = c + 5.5 * u * Vec2::angled(-2.6);
            line(&[
                start + vec2(-0.5 * u, -3.0 * u),
                start,
                start + vec2(3.0 * u, 0.3 * u),
            ]);
        }
        Icon::Export => {
            line(&[
                pt(5.0, 6.5),
                pt(3.5, 6.5),
                pt(3.5, 14.0),
                pt(12.5, 14.0),
                pt(12.5, 6.5),
                pt(11.0, 6.5),
            ]);
            line(&[pt(8.0, 10.0), pt(8.0, 1.8)]);
            line(&[pt(5.5, 4.3), pt(8.0, 1.8), pt(10.5, 4.3)]);
        }
    }
}

fn arc(c: Pos2, r: f32, a0: f32, a1: f32, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            c + r * Vec2::angled(a)
        })
        .collect()
}

/// The keyboard focus ring of a custom-drawn control: the accent, inside its `rect` (white on
/// a solid accent fill), so Tab shows where it is without moving anything.
pub fn focus_ring(ui: &egui::Ui, rect: Rect, resp: &egui::Response, on_accent: bool) {
    if resp.has_focus() {
        let c = if on_accent { Color32::WHITE } else { accent() };
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            RADIUS,
            Stroke::new(1.5, c),
            egui::StrokeKind::Inside,
        );
    }
}

/// An icon button for permanent chrome (the panels' top rows and the canvas's floating
/// controls): drawn in the secondary colour and only in the text colour while hovered or
/// focused, so the chrome stays quiet around the canvas. `on` (an open panel's toggle) is a
/// neutral pressed fill with the icon in the text colour, like a pushed-in macOS toolbar
/// button: the accent is kept for the current tool, the selection and keyboard focus.
pub fn quiet_icon_button(ui: &mut egui::Ui, icon: Icon, size: f32, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = pal(ui);
        let hovered = (resp.hovered() || resp.has_focus()) && ui.is_enabled();
        let fill = if resp.is_pointer_button_down_on() || on {
            p.pressed
        } else if hovered {
            p.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, RADIUS, fill);
        let color = if !ui.is_enabled() {
            p.tertiary
        } else if on || hovered {
            p.text
        } else {
            p.secondary
        };
        paint_icon(ui.painter(), icon_rect(rect), icon, color);
        focus_ring(ui, rect, &resp, false);
    }
    resp
}

/// A borderless icon button: subtle hover/pressed fill; when `selected` (the current tool, an
/// alignment or layout toggle that is on) the accent tint of `selected_fill` with the icon in
/// the accent's ink, as a selected layer row is tinted, rather than a solid accent block.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, size: f32, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = pal(ui);
        let enabled = ui.is_enabled();
        let fill = if selected {
            selected_fill(ui)
        } else if resp.is_pointer_button_down_on() {
            p.pressed
        } else if resp.hovered() && enabled {
            p.hover
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, RADIUS, fill);
        let color = if !enabled {
            p.tertiary
        } else if selected {
            accent_ink(ui.visuals().dark_mode)
        } else {
            p.text
        };
        paint_icon(ui.painter(), icon_rect(rect), icon, color);
        focus_ring(ui, rect, &resp, false);
    }
    resp
}
