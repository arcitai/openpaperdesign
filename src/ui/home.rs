//! The projects home screen: a recents grid with thumbnails, plus the trash.

use crate::project::{Entry, TrashEntry};
use crate::theme::{self, Icon};
use egui::{Color32, RichText, Ui};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq)]
pub enum HomeAction {
    New,
    Import,
    Open(PathBuf),
    Rename(PathBuf, String),
    Duplicate(PathBuf),
    Trash(PathBuf),
    Restore(PathBuf),
    DeleteForever(PathBuf),
    EmptyTrash,
}

#[derive(Default)]
pub struct HomeView {
    pub entries: Vec<Entry>,
    pub trash: Vec<TrashEntry>,
    pub show_trash: bool,
    pub renaming: Option<(PathBuf, String)>,
    /// Requests for the app, drained after each frame.
    pub actions: Vec<HomeAction>,
    /// "Now" for relative timestamps (unix ms); fixed in screenshots.
    pub now_ms: u64,
    pub root: PathBuf,
    pub query: String,
    pub list_view: bool,
    trash_thumbs: HashMap<PathBuf, PathBuf>,
    thumbs: HashMap<PathBuf, (Option<SystemTime>, egui::TextureHandle)>,
}

const GAP: f32 = 12.0;
const CARD_MIN_W: f32 = 208.0;

impl HomeView {
    /// Re-read the project list and trash from the store (keeps cached thumbnails).
    pub fn refresh(&mut self, store: &crate::project::Store, now_ms: u64) {
        self.root = store.root.clone();
        self.entries = store.list();
        self.trash = store.list_trash();
        self.now_ms = now_ms;
        // Refresh performs the filesystem checks once. Drawing/filtering the library does
        // not repeatedly stat every thumbnail while the pointer moves around Home.
        self.thumbs.retain(|path, (time, _)| {
            std::fs::metadata(path).and_then(|m| m.modified()).ok() == *time
        });
        self.trash_thumbs = self
            .trash
            .iter()
            .filter_map(|entry| {
                let path = entry.dir.join(crate::project::THUMBNAIL);
                path.is_file().then(|| (entry.dir.clone(), path))
            })
            .collect();
    }

    pub fn load(store: &crate::project::Store, now_ms: u64) -> HomeView {
        let mut hv = HomeView::default();
        hv.refresh(store, now_ms);
        hv
    }

    fn thumb(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
        if let Some((_, handle)) = self.thumbs.get(path) {
            return Some(handle.clone());
        }
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let img = image::open(path).ok()?.to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        let h = ctx.load_texture(path.to_string_lossy(), ci, egui::TextureOptions::LINEAR);
        self.thumbs.insert(path.to_path_buf(), (mtime, h.clone()));
        Some(h)
    }
}

/// "Edited 5 minutes ago" style relative time.
pub fn relative_time(then_ms: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(then_ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => plural(s / 60, "minute"),
        3600..=86_399 => plural(s / 3600, "hour"),
        86_400..=2_591_999 => plural(s / 86_400, "day"),
        _ => plural(s / 2_592_000, "month"),
    }
}

fn plural(n: u64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{n} {unit}s ago")
    }
}

// SVG is the identity source; these committed derivatives share its exact geometry/colour.
fn brand(ui: &mut Ui) {
    let dark = ui.visuals().dark_mode;
    let id = egui::Id::new(("opd-brand", dark));
    let cached = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let png: &[u8] = if dark {
            include_bytes!("../../assets/brand/home-logo-dark.png")
        } else {
            include_bytes!("../../assets/brand/home-logo-light.png")
        };
        let image = image::load_from_memory(png)
            .expect("validated bundled brand PNG")
            .to_rgba8();
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        let texture = ui.ctx().load_texture(
            if dark {
                "opd-brand-dark"
            } else {
                "opd-brand-light"
            },
            pixels,
            egui::TextureOptions::LINEAR,
        );
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    ui.add(egui::Image::new((texture.id(), egui::vec2(24.0, 24.0))));
}

fn primary(ui: &mut Ui, text: &str) -> egui::Response {
    let p = theme::pal(ui);
    ui.scope(|ui| {
        let w = &mut ui.style_mut().visuals.widgets;
        for v in [&mut w.inactive, &mut w.hovered, &mut w.active] {
            v.bg_fill = p.text;
            v.weak_bg_fill = p.text;
            v.fg_stroke = egui::Stroke::new(1.0, p.window);
        }
        w.hovered.bg_fill = theme::blend(p.window, p.text.gamma_multiply(0.88));
        w.hovered.weak_bg_fill = w.hovered.bg_fill;
        ui.add_sized([100.0, 30.0], egui::Button::new(text))
    })
    .inner
}

pub fn draw(ui: &mut Ui, hv: &mut HomeView, native_chrome: bool, translucent: bool) {
    let p = theme::pal(ui);
    let wide = ui.available_width() >= 760.0;
    if wide {
        egui::Panel::left("home-sidebar")
            .resizable(false)
            .exact_size(200.0)
            .frame(
                egui::Frame::NONE
                    .fill(if translucent {
                        Color32::TRANSPARENT
                    } else {
                        p.sidebar
                    })
                    .inner_margin(egui::Margin::symmetric(12, 12)),
            )
            .show(ui, |ui| {
                if native_chrome {
                    ui.add_space(theme::TOOLBAR_H - 12.0);
                }
                ui.horizontal(|ui| {
                    brand(ui);
                    ui.label(RichText::new("OpenPaperDesign").font(theme::semibold(theme::TITLE)));
                });
                ui.add_space(20.0);
                if sidebar_item(ui, Icon::Grid, "Recents", None, !hv.show_trash) {
                    hv.show_trash = false;
                }
                if sidebar_item(
                    ui,
                    Icon::Trash,
                    "Trash",
                    (!hv.trash.is_empty()).then(|| hv.trash.len().to_string()),
                    hv.show_trash,
                ) {
                    hv.show_trash = true;
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(
                        RichText::new("Saved on this device")
                            .size(theme::SECONDARY)
                            .color(p.secondary),
                    )
                    .on_hover_text(hv.root.display().to_string());
                });
                let r = ui.max_rect();
                ui.painter().vline(
                    r.right() + 12.0,
                    r.y_range(),
                    egui::Stroke::new(1.0, p.separator),
                );
            });
    }
    egui::Panel::top("home-toolbar")
        .exact_size(theme::TOOLBAR_H)
        .frame(
            egui::Frame::NONE
                .fill(p.window)
                .inner_margin(egui::Margin::symmetric(20, 0)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if !wide && native_chrome {
                    ui.add_space(theme::TRAFFIC_LIGHTS_W);
                }
                if wide {
                    ui.label(
                        RichText::new(if hv.show_trash { "Trash" } else { "Recents" })
                            .font(theme::semibold(17.0)),
                    );
                } else {
                    ui.menu_button(
                        if hv.show_trash {
                            "Trash ▾"
                        } else {
                            "Recents ▾"
                        },
                        |ui| {
                            ui.label(
                                RichText::new("OpenPaperDesign")
                                    .font(theme::semibold(theme::TITLE)),
                            );
                            ui.separator();
                            if ui.selectable_label(!hv.show_trash, "Recents").clicked() {
                                hv.show_trash = false;
                                ui.close();
                            }
                            if ui.selectable_label(hv.show_trash, "Trash").clicked() {
                                hv.show_trash = true;
                                ui.close();
                            }
                        },
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if hv.show_trash {
                        if ui
                            .add_enabled(!hv.trash.is_empty(), egui::Button::new("Empty Trash"))
                            .clicked()
                        {
                            hv.actions.push(HomeAction::EmptyTrash);
                        }
                    } else if primary(ui, "New project").clicked() {
                        hv.actions.push(HomeAction::New);
                    }
                });
            });
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(p.window).inner_margin(20))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            ui.label(
                RichText::new(if hv.show_trash {
                    "Restore a project or remove it permanently."
                } else {
                    "Your projects, saved as you work."
                })
                .size(theme::SECONDARY)
                .color(p.secondary),
            );
            ui.horizontal(|ui| {
                let width =
                    (ui.available_width() - if hv.show_trash { 0.0 } else { 164.0 }).max(40.0);
                let response = ui.add_sized(
                    [width, 30.0],
                    egui::TextEdit::singleline(&mut hv.query)
                        .hint_text("Search projects")
                        .id_salt("home-search"),
                );
                if (response.has_focus() || response.lost_focus())
                    && ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    hv.query.clear();
                }
                if !hv.show_trash {
                    if theme::icon_button(ui, Icon::List, 30.0, hv.list_view)
                        .on_hover_text("List")
                        .clicked()
                    {
                        hv.list_view = true;
                    }
                    if theme::icon_button(ui, Icon::Grid, 30.0, !hv.list_view)
                        .on_hover_text("Grid")
                        .clicked()
                    {
                        hv.list_view = false;
                    }
                    if ui
                        .add_sized([68.0, 30.0], egui::Button::new("Import…"))
                        .clicked()
                    {
                        hv.actions.push(HomeAction::Import);
                    }
                }
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    if hv.show_trash {
                        trash_grid(ui, hv);
                    } else {
                        recents_grid(ui, hv);
                    }
                });
        });
}

fn sidebar_item(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    badge: Option<String>,
    selected: bool,
) -> bool {
    let p = theme::pal(ui);
    let (row, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(row, theme::RADIUS, p.pressed);
    } else if resp.hovered() {
        ui.painter().rect_filled(row, theme::RADIUS, p.hover);
    }
    let ir = egui::Rect::from_center_size(
        egui::pos2(row.left() + 16.0, row.center().y),
        egui::vec2(15.0, 15.0),
    );
    theme::paint_icon(
        ui.painter(),
        ir,
        icon,
        if selected { p.text } else { p.secondary },
    );
    ui.painter().text(
        egui::pos2(row.left() + 32.0, row.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(theme::BODY),
        p.text,
    );
    if let Some(b) = badge {
        ui.painter().text(
            egui::pos2(row.right() - 8.0, row.center().y),
            egui::Align2::RIGHT_CENTER,
            b,
            egui::FontId::proportional(theme::SECONDARY),
            p.secondary,
        );
    }
    theme::focus_ring(ui, row, &resp, false);
    resp.clicked()
}

fn grid_metrics(width: f32) -> (usize, f32) {
    let cols = ((width + GAP) / (CARD_MIN_W + GAP)).floor().max(1.0) as usize;
    (
        cols,
        ((width - GAP * (cols - 1) as f32) / cols as f32)
            .floor()
            .max(1.0),
    )
}
fn matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(&query.trim().to_lowercase())
}

fn recents_grid(ui: &mut Ui, hv: &mut HomeView) {
    let p = theme::pal(ui);
    if hv.entries.is_empty() {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("No projects yet").font(theme::semibold(17.0)));
            ui.add_space(4.0);
            ui.label(
                RichText::new("Create a project or import a canvas to get started.")
                    .color(p.secondary),
            );
            ui.add_space(12.0);
            if primary(ui, "New project").clicked() {
                hv.actions.push(HomeAction::New);
            }
        });
        return;
    }
    let (cols, width) = grid_metrics(ui.available_width());
    let entries: Vec<_> = hv
        .entries
        .iter()
        .filter(|e| matches(&e.manifest.name, &hv.query))
        .cloned()
        .collect();
    if entries.is_empty() {
        ui.label(RichText::new("No projects match your search.").color(p.secondary));
    }
    if hv.list_view {
        for entry in &entries {
            project_row(ui, hv, entry);
        }
        return;
    }
    for row in entries.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for e in row {
                project_card(ui, hv, e, width);
            }
        });
        ui.add_space(GAP);
    }
}

fn card_frame(
    ui: &mut Ui,
    hv: &mut HomeView,
    thumb: Option<&Path>,
    id: egui::Id,
    width: f32,
) -> egui::Response {
    let p = theme::pal(ui);
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, (width * 0.625).floor()),
        egui::Sense::hover(),
    );
    let resp = ui.interact(rect, id, egui::Sense::click());
    let hovered = resp.hovered();
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, p.field);
    if let Some(t) = thumb.and_then(|t| hv.thumb(ui.ctx(), t)) {
        // Fit the thumbnail inside the card, centered.
        let [tw, th] = t.size();
        let s = ((rect.width() - 16.0) / tw as f32)
            .min((rect.height() - 16.0) / th as f32)
            .min(1.0 / ui.ctx().pixels_per_point());
        let r =
            egui::Rect::from_center_size(rect.center(), egui::vec2(tw as f32 * s, th as f32 * s));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        theme::paint_icon(
            ui.painter(),
            egui::Rect::from_center_size(rect.center(), egui::vec2(28.0, 28.0)),
            Icon::Page,
            p.secondary,
        );
    }
    ui.painter().rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(
            1.0,
            if hovered {
                if ui.visuals().dark_mode {
                    Color32::from_rgb(0x48, 0x48, 0x4c)
                } else {
                    Color32::from_rgb(0xd6, 0xd6, 0xd9)
                }
            } else {
                p.separator
            },
        ),
        egui::StrokeKind::Inside,
    );
    theme::focus_ring(ui, rect, &resp, false);
    resp
}

fn project_card(ui: &mut Ui, hv: &mut HomeView, e: &Entry, width: f32) {
    let p = theme::pal(ui);
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 2.0;
        let id = egui::Id::new(("card", &e.dir));
        let resp = card_frame(ui, hv, e.thumbnail.as_deref(), id, width);
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &e.manifest.name)
        });
        if resp.double_clicked()
            || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            hv.actions.push(HomeAction::Open(e.dir.clone()));
        }
        resp.context_menu(|ui| card_menu(ui, hv, e));
        ui.add_space(6.0);
        let renaming = hv.renaming.as_ref().is_some_and(|(d, _)| *d == e.dir);
        if renaming {
            let (_, name) = hv.renaming.as_mut().unwrap();
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(width));
            if r.lost_focus() {
                let (dir, name) = hv.renaming.take().unwrap();
                if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && name.trim() != e.manifest.name
                {
                    hv.actions.push(HomeAction::Rename(dir, name));
                }
            } else if !r.has_focus() {
                r.request_focus();
            }
        } else {
            ui.horizontal(|ui| {
                let name = ui.add(
                    egui::Label::new(
                        RichText::new(&e.manifest.name).font(theme::medium(theme::BODY)),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                );
                if name.double_clicked() {
                    hv.renaming = Some((e.dir.clone(), e.manifest.name.clone()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let more =
                        theme::icon_button(ui, Icon::More, 20.0, false).on_hover_text("More");
                    egui::Popup::menu(&more).show(|ui| card_menu(ui, hv, e));
                });
            });
        }
        let pages = e.manifest.pages.len();
        ui.label(
            RichText::new(format!(
                "Edited {} · {} page{}",
                relative_time(e.manifest.modified, hv.now_ms),
                pages,
                if pages == 1 { "" } else { "s" }
            ))
            .size(theme::SECONDARY)
            .color(p.secondary),
        );
    });
}

fn project_row(ui: &mut Ui, hv: &mut HomeView, entry: &Entry) {
    let p = theme::pal(ui);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::hover());
    let response = ui.interact(
        rect,
        egui::Id::new(("project-row", &entry.dir)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &entry.manifest.name)
    });
    if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, p.hover);
    }
    theme::focus_ring(ui, rect, &response, false);
    if response.double_clicked()
        || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
    {
        hv.actions.push(HomeAction::Open(entry.dir.clone()));
    }
    response.context_menu(|ui| card_menu(ui, hv, entry));
    theme::paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 16.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        ),
        Icon::Page,
        p.secondary,
    );
    let text_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(36.0, 5.0),
        rect.max - egui::vec2(44.0, 5.0),
    );
    if hv
        .renaming
        .as_ref()
        .is_some_and(|(dir, _)| *dir == entry.dir)
    {
        ui.scope_builder(egui::UiBuilder::new().max_rect(text_rect), |ui| {
            let (_, name) = hv.renaming.as_mut().unwrap();
            let response =
                ui.add_sized([text_rect.width(), 28.0], egui::TextEdit::singleline(name));
            if response.lost_focus() {
                let (dir, name) = hv.renaming.take().unwrap();
                if !ui.input(|i| i.key_pressed(egui::Key::Escape))
                    && name.trim() != entry.manifest.name
                {
                    hv.actions.push(HomeAction::Rename(dir, name));
                }
            } else if !response.has_focus() {
                response.request_focus();
            }
        });
    } else {
        let mut job = egui::text::LayoutJob::simple(
            entry.manifest.name.clone(),
            theme::medium(theme::BODY),
            p.text,
            text_rect.width(),
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(text_rect.min, galley, p.text);
        ui.painter().text(
            text_rect.min + egui::vec2(0.0, 17.0),
            egui::Align2::LEFT_TOP,
            format!(
                "Edited {}",
                relative_time(entry.manifest.modified, hv.now_ms)
            ),
            egui::FontId::proportional(theme::SECONDARY),
            p.secondary,
        );
        response.clone().on_hover_text(&entry.manifest.name);
    }
    let more_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 36.0, rect.center().y - 14.0),
        egui::vec2(28.0, 28.0),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(more_rect), |ui| {
        let more = theme::icon_button(ui, Icon::More, 28.0, false).on_hover_text("More");
        egui::Popup::menu(&more).show(|ui| card_menu(ui, hv, entry));
    });
}

fn card_menu(ui: &mut Ui, hv: &mut HomeView, e: &Entry) {
    if ui.button("Open").clicked() {
        hv.actions.push(HomeAction::Open(e.dir.clone()));
        ui.close();
    }
    if ui.button("Rename").clicked() {
        hv.renaming = Some((e.dir.clone(), e.manifest.name.clone()));
        ui.close();
    }
    if ui.button("Duplicate").clicked() {
        hv.actions.push(HomeAction::Duplicate(e.dir.clone()));
        ui.close();
    }
    ui.separator();
    if ui.button("Move to Trash").clicked() {
        hv.actions.push(HomeAction::Trash(e.dir.clone()));
        ui.close();
    }
}

fn trash_grid(ui: &mut Ui, hv: &mut HomeView) {
    let p = theme::pal(ui);
    if hv.trash.is_empty() {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Trash is empty").font(theme::semibold(17.0)));
            ui.label(
                RichText::new("Deleted projects stay here until you empty the trash.")
                    .color(p.secondary),
            );
        });
        return;
    }
    let (cols, width) = grid_metrics(ui.available_width());
    let entries: Vec<_> = hv
        .trash
        .iter()
        .filter(|e| matches(&e.manifest.name, &hv.query))
        .cloned()
        .collect();
    if entries.is_empty() {
        ui.label(RichText::new("No projects match your search.").color(p.secondary));
    }
    for row in entries.chunks(cols) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for e in row {
                ui.vertical(|ui| {
                    ui.set_width(width);
                    let thumb = hv.trash_thumbs.get(&e.dir).cloned();
                    let resp = card_frame(ui, hv, thumb.as_deref(), egui::Id::new(&e.dir), width);
                    resp.context_menu(|ui| {
                        if ui.button("Restore").clicked() {
                            hv.actions.push(HomeAction::Restore(e.dir.clone()));
                            ui.close();
                        }
                        if ui.button("Delete permanently").clicked() {
                            hv.actions.push(HomeAction::DeleteForever(e.dir.clone()));
                            ui.close();
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&e.manifest.name).font(theme::medium(theme::BODY)),
                            )
                            .truncate(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::icon_button(ui, Icon::Restore, 20.0, false)
                                .on_hover_text("Restore")
                                .clicked()
                            {
                                hv.actions.push(HomeAction::Restore(e.dir.clone()));
                            }
                        });
                    });
                    ui.label(
                        RichText::new(format!("Deleted {}", relative_time(e.deleted, hv.now_ms)))
                            .size(theme::SECONDARY)
                            .color(p.secondary),
                    );
                });
            }
        });
        ui.add_space(GAP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_names_and_keeps_cards_inside_narrow_widths() {
        assert!(matches("Kundens Ønsker", "øNS"));
        assert!(matches("Board", "  "));
        assert!(!matches("Board", "notes"));
        for width in [160.0, 248.0, 424.0, 640.0, 1232.0, 2000.0] {
            let (cols, card) = grid_metrics(width);
            assert!(cols >= 1 && card > 0.0);
            assert!(cols as f32 * card + (cols - 1) as f32 * GAP <= width);
        }
    }

    #[test]
    fn relative_times() {
        let now = 10_000_000_000;
        assert_eq!(relative_time(now - 5_000, now), "just now");
        assert_eq!(relative_time(now - 60_000, now), "1 minute ago");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3 hours ago");
        assert_eq!(relative_time(now - 2 * 86_400_000, now), "2 days ago");
        assert_eq!(relative_time(now + 1, now), "just now");
    }
}
