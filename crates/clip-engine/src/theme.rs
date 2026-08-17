use eframe::egui::{
    self, Align, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame,
    Layout, Margin, Pos2, Rect, RichText, Sense, Shadow, Shape, Stroke, Ui, UiBuilder, Vec2,
    Visuals,
};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(10, 11, 13);
pub const PANEL: Color32 = Color32::from_rgb(16, 18, 22);
pub const CARD: Color32 = Color32::from_rgb(22, 25, 31);
pub const CARD_HOVER: Color32 = Color32::from_rgb(28, 32, 40);
pub const LINE: Color32 = Color32::from_rgb(42, 48, 58);
pub const ACCENT: Color32 = Color32::from_rgb(194, 247, 64);
pub const ACCENT_DIM: Color32 = Color32::from_rgba_premultiplied(43, 54, 14, 56);
pub const TEXT: Color32 = Color32::from_rgb(236, 233, 226);
pub const MUTED: Color32 = Color32::from_rgb(148, 154, 164);
pub const DANGER: Color32 = Color32::from_rgb(232, 112, 104);
pub const OK: Color32 = Color32::from_rgb(110, 186, 122);
pub const INK: Color32 = Color32::from_rgb(12, 16, 10);

pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

pub fn apply(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "plex".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/IBMPlexSans-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "plex-medium".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/IBMPlexSans-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        "plex-mono".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/IBMPlexMono-Regular.ttf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "plex".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "plex-mono".into());
    fonts.families.insert(
        medium(),
        vec!["plex-medium".into(), "plex".into(), "Hack".into()],
    );
    ctx.set_fonts(fonts);

    let mut visuals = Visuals::dark();
    visuals.dark_mode = true;
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = CARD;
    visuals.faint_bg_color = CARD;
    visuals.override_text_color = Some(TEXT);
    visuals.window_stroke = Stroke::new(1.0, LINE);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.window_shadow = Shadow::NONE;
    visuals.selection.bg_fill = ACCENT_DIM;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.weak_bg_fill = CARD;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(4);
    visuals.widgets.hovered.bg_fill = CARD_HOVER;
    visuals.widgets.hovered.weak_bg_fill = CARD_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.55));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(4);
    visuals.widgets.active.bg_fill = Color32::from_rgb(36, 40, 50);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.active.corner_radius = CornerRadius::same(4);
    visuals.widgets.open.bg_fill = CARD_HOVER;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.open.corner_radius = CornerRadius::same(4);
    ctx.set_visuals(visuals);

    ctx.style_mut_of(ctx.theme(), |style| {
        style
            .text_styles
            .insert(egui::TextStyle::Heading, FontId::new(20.0, medium()));
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::new(14.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        );
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.indent = 14.0;
        style.spacing.scroll.floating_allocated_width = style.spacing.scroll.bar_width + 4.0;
    });
}

pub fn top_frame() -> Frame {
    Frame::NONE
        .fill(PANEL)
        .inner_margin(Margin::symmetric(16, 10))
        .stroke(Stroke::new(1.0, LINE))
}

pub fn side_frame() -> Frame {
    Frame::NONE
        .fill(PANEL)
        .inner_margin(Margin::symmetric(12, 12))
        .stroke(Stroke::new(1.0, LINE))
}

pub fn central_frame() -> Frame {
    Frame::NONE.fill(BG).inner_margin(Margin::symmetric(16, 14))
}

pub fn card() -> Frame {
    Frame::NONE
        .fill(CARD)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(6)
        .inner_margin(Margin::same(12))
}

pub fn inset() -> Frame {
    Frame::NONE
        .fill(BG)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(10, 8))
}

pub fn published_tick_overlay(ui: &Ui, thumb: Rect, count: usize) -> egui::Response {
    let size = 15.0;
    let pad = 3.0;
    let badge = Rect::from_center_size(
        Pos2::new(
            thumb.right() - pad - size * 0.5,
            thumb.bottom() - pad - size * 0.5,
        ),
        Vec2::splat(size),
    );
    let painter = ui.painter();
    painter.circle_filled(
        badge.center(),
        size * 0.5,
        Color32::from_rgba_unmultiplied(12, 16, 14, 210),
    );
    let stroke = Stroke::new(1.6, OK.gamma_multiply(0.92));
    let c = badge.center();
    painter.line_segment(
        [
            Pos2::new(c.x - 3.5, c.y + 0.4),
            Pos2::new(c.x - 1.1, c.y + 2.8),
        ],
        stroke,
    );
    painter.line_segment(
        [
            Pos2::new(c.x - 1.1, c.y + 2.8),
            Pos2::new(c.x + 3.7, c.y - 2.7),
        ],
        stroke,
    );
    ui.interact(badge, ui.id().with("published-tick"), Sense::hover())
        .on_hover_text(if count == 1 {
            "Uploaded".into()
        } else {
            format!("{count} published versions")
        })
}

pub fn progress_bar(ui: &mut Ui, progress: f32) {
    let height = 10.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(5), Color32::from_rgb(14, 16, 20));
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(5),
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
    let filled = rect.width() * progress.clamp(0.0, 1.0);
    if filled > 0.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(
                rect.min,
                Vec2::new(filled.max(6.0).min(rect.width()), rect.height()),
            ),
            CornerRadius::same(5),
            ACCENT,
        );
    }
}

pub fn fit_contain(max_w: f32, max_h: f32, aspect: f32) -> Vec2 {
    let aspect = aspect.max(0.01);
    let mut width = max_w.max(2.0);
    let mut height = width / aspect;
    if height > max_h {
        height = max_h.max(2.0);
        width = height * aspect;
    }
    Vec2::new(width, height)
}

pub fn poster_play_overlay(ui: &Ui, rect: Rect) {
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(7),
        Color32::from_rgba_unmultiplied(8, 9, 11, 118),
    );
    let center = rect.center();
    ui.painter().circle_filled(
        center,
        30.0,
        Color32::from_rgba_unmultiplied(16, 14, 10, 230),
    );
    ui.painter()
        .circle_stroke(center, 30.0, Stroke::new(1.5, ACCENT));
    ui.painter().add(Shape::convex_polygon(
        vec![
            Pos2::new(center.x - 8.0, center.y - 12.0),
            Pos2::new(center.x - 8.0, center.y + 12.0),
            Pos2::new(center.x + 14.0, center.y),
        ],
        ACCENT,
        Stroke::NONE,
    ));
}

pub fn buffering_overlay(ui: &Ui, rect: Rect) {
    let time = ui.ctx().input(|input| input.time) as f32;
    let center = rect.center();
    ui.painter().rect_filled(
        rect,
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(8, 9, 11, 88),
    );
    ui.painter().circle_filled(
        center,
        26.0,
        Color32::from_rgba_unmultiplied(12, 13, 16, 210),
    );
    ui.painter()
        .circle_stroke(center, 26.0, Stroke::new(1.0, LINE));
    let radius = 11.0;
    let start = time * 5.2;
    let points = (0..16)
        .map(|index| {
            let angle = start + index as f32 * 0.16;
            Pos2::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect::<Vec<_>>();
    ui.painter()
        .add(Shape::line(points, Stroke::new(2.4, ACCENT)));
    ui.painter().text(
        Pos2::new(center.x, center.y + 42.0),
        egui::Align2::CENTER_CENTER,
        "Buffering",
        FontId::new(12.0, medium()),
        MUTED,
    );
    ui.ctx().request_repaint();
}

pub fn window_drop_overlay(ctx: &egui::Context) {
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("window-drop-overlay"),
    ));
    painter.rect_filled(
        screen,
        CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(10, 11, 13, 138),
    );
    let inset = screen.shrink(14.0);
    painter.rect_filled(inset, CornerRadius::same(10), ACCENT_DIM);
    paint_dashed_rect(&painter, inset.shrink(1.0), ACCENT);
    painter.text(
        screen.center() - Vec2::new(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        "Drop recordings to import",
        FontId::new(22.0, medium()),
        ACCENT,
    );
    painter.text(
        screen.center() + Vec2::new(0.0, 16.0),
        egui::Align2::CENTER_CENTER,
        "Anywhere in the window works",
        FontId::new(13.0, FontFamily::Proportional),
        MUTED,
    );
}

pub fn import_drop_zone(ui: &mut Ui, hovering_files: bool, height: f32) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let hovered = response.hovered() || hovering_files;
    let active = response.is_pointer_button_down_on();
    let radius = CornerRadius::same(6);
    let bg = if hovering_files {
        ACCENT_DIM
    } else if active {
        Color32::from_rgb(36, 40, 50)
    } else if hovered {
        CARD_HOVER
    } else {
        CARD
    };
    let stroke = if hovering_files || active {
        ACCENT
    } else if hovered {
        ACCENT.gamma_multiply(0.7)
    } else {
        LINE
    };

    ui.painter().rect_filled(rect, radius, bg);
    paint_dashed_rect(ui.painter(), rect.shrink(1.0), stroke);

    let title = if hovering_files {
        "Drop to import"
    } else {
        "Drop recordings here"
    };
    let hint = if hovering_files {
        "Release to add them to the library"
    } else {
        "or click to browse"
    };
    ui.painter().text(
        rect.center() - Vec2::new(0.0, 8.0),
        egui::Align2::CENTER_CENTER,
        title,
        FontId::new(13.5, medium()),
        if hovering_files { ACCENT } else { TEXT },
    );
    ui.painter().text(
        rect.center() + Vec2::new(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        hint,
        FontId::new(11.5, FontFamily::Proportional),
        MUTED,
    );

    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Choose recordings to import")
}

fn paint_dashed_rect(painter: &egui::Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.15, color);
    let dash = 5.5;
    let gap = 3.5;
    let corners = [
        [rect.left_top(), rect.right_top()],
        [rect.right_top(), rect.right_bottom()],
        [rect.right_bottom(), rect.left_bottom()],
        [rect.left_bottom(), rect.left_top()],
    ];
    for points in corners {
        painter.add(Shape::dashed_line(&points, stroke, dash, gap));
    }
}

pub fn library_menu_button(ui: &mut Ui, open: bool) -> egui::Response {
    let size = Vec2::splat(32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let active = response.is_pointer_button_down_on();
    let hover_t = ui
        .ctx()
        .animate_bool_with_time(response.id.with("hover"), hovered, 0.14);

    let radius = CornerRadius::same(8);
    let bg = if active {
        Color32::from_rgb(36, 40, 50)
    } else if hovered {
        CARD_HOVER
    } else {
        CARD
    };
    let stroke = if hovered || active || open {
        ACCENT.gamma_multiply(0.7)
    } else {
        LINE
    };
    ui.painter().rect_filled(rect, radius, bg);
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );

    let mut center = rect.center();
    if hover_t > 0.0 {
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let raw = (pointer - center) / 42.0;
            center += Vec2::new(raw.x.clamp(-1.0, 1.0), raw.y.clamp(-1.0, 1.0)) * (2.8 * hover_t);
        }
        ui.ctx().request_repaint();
    }

    let color = if hovered || open { ACCENT } else { TEXT };
    let length = 14.0 + hover_t * 1.4;
    let thickness = 2.15;
    let gap = 5.0 + hover_t * 0.6;

    paint_menu_bar(
        ui.painter(),
        center + Vec2::new(0.0, -gap),
        length,
        thickness,
        0.0,
        color,
    );
    paint_menu_bar(ui.painter(), center, length, thickness, 0.0, color);
    paint_menu_bar(
        ui.painter(),
        center + Vec2::new(0.0, gap),
        length,
        thickness,
        0.0,
        color,
    );

    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if open { "Hide library" } else { "Show library" })
}

fn paint_menu_bar(
    painter: &egui::Painter,
    center: Pos2,
    length: f32,
    thickness: f32,
    angle: f32,
    color: Color32,
) {
    if length <= 0.5 || color.a() == 0 {
        return;
    }
    let half = Vec2::angled(angle) * (length * 0.5);
    let start = center - half;
    let end = center + half;
    painter.line_segment([start, end], Stroke::new(thickness, color));
    painter.circle_filled(start, thickness * 0.5, color);
    painter.circle_filled(end, thickness * 0.5, color);
}

pub fn folder_path_field(ui: &mut Ui, path: &str) -> egui::Response {
    let height = (ui.text_style_height(&egui::TextStyle::Button)
        + ui.spacing().button_padding.y * 2.0)
        .max(ui.spacing().interact_size.y);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let hovered = response.hovered();
    let active = response.is_pointer_button_down_on();
    let radius = CornerRadius::same(4);
    let bg = if active {
        Color32::from_rgb(14, 16, 20)
    } else if hovered {
        Color32::from_rgb(18, 20, 25)
    } else {
        BG
    };
    let stroke = if active {
        ACCENT
    } else if hovered {
        ACCENT.gamma_multiply(0.7)
    } else {
        LINE
    };
    ui.painter().rect_filled(rect, radius, bg);
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center))
            .sense(Sense::hover()),
        |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
            let icon_color = if hovered || active { ACCENT } else { MUTED };
            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_folder_icon(ui.painter(), icon_rect, icon_color);
            let display = if path.is_empty() {
                "Choose inbox folder"
            } else {
                path
            };
            ui.add(
                egui::Label::new(RichText::new(display).monospace().size(12.0).color(
                    if path.is_empty() {
                        MUTED
                    } else if hovered {
                        TEXT
                    } else {
                        Color32::from_rgb(196, 200, 208)
                    },
                ))
                .truncate()
                .selectable(false),
            );
        },
    );
    let tooltip = if path.is_empty() {
        "Choose inbox folder"
    } else {
        path
    };
    response.on_hover_text(tooltip)
}

pub fn hotkey_button(ui: &mut Ui, key: &str, label: &str, key_first: bool) -> egui::Response {
    let height = (ui.text_style_height(&egui::TextStyle::Button)
        + ui.spacing().button_padding.y * 2.0)
        .max(ui.spacing().interact_size.y);
    let key_galley = ui
        .painter()
        .layout_no_wrap(key.to_owned(), FontId::monospace(11.0), TEXT);
    let label_galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::new(13.5, FontFamily::Proportional),
        TEXT,
    );
    let key_size = Vec2::new(
        (key_galley.size().x + 10.0).max(22.0),
        (key_galley.size().y + 5.0).max(18.0),
    );
    let pad = ui.spacing().button_padding;
    let gap = 8.0;
    let width = pad.x * 2.0 + key_size.x + gap + label_galley.size().x;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let hovered = response.hovered();
    let active = response.is_pointer_button_down_on();
    let radius = CornerRadius::same(4);
    let bg = if active {
        Color32::from_rgb(36, 40, 50)
    } else if hovered {
        CARD_HOVER
    } else {
        CARD
    };
    let stroke = if hovered || active {
        ACCENT.gamma_multiply(0.7)
    } else {
        LINE
    };
    ui.painter().rect_filled(rect, radius, bg);
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink2(Vec2::new(pad.x, 0.0));
    let (key_rect, label_pos) = if key_first {
        let key_rect = Rect::from_min_size(
            Pos2::new(inner.left(), inner.center().y - key_size.y * 0.5),
            key_size,
        );
        let label_pos = Pos2::new(
            key_rect.right() + gap,
            inner.center().y - label_galley.size().y * 0.5,
        );
        (key_rect, label_pos)
    } else {
        let key_rect = Rect::from_min_size(
            Pos2::new(
                inner.right() - key_size.x,
                inner.center().y - key_size.y * 0.5,
            ),
            key_size,
        );
        let label_pos = Pos2::new(inner.left(), inner.center().y - label_galley.size().y * 0.5);
        (key_rect, label_pos)
    };
    paint_keycap(
        ui.painter(),
        key_rect,
        key_galley,
        if hovered || active { ACCENT } else { TEXT },
    );
    ui.painter().galley(label_pos, label_galley, TEXT);
    response.on_hover_text(format!("{label}  ·  {key}"))
}

fn paint_keycap(
    painter: &egui::Painter,
    rect: Rect,
    galley: std::sync::Arc<egui::Galley>,
    color: Color32,
) {
    let lip = 2.0;
    let base = Color32::from_rgb(18, 20, 24);
    let face = Color32::from_rgb(38, 43, 52);
    painter.rect_filled(rect, CornerRadius::same(3), base);
    let face_rect = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.bottom() - lip));
    painter.rect_filled(face_rect, CornerRadius::same(3), face);
    painter.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0, Color32::from_rgb(58, 64, 76)),
        egui::StrokeKind::Inside,
    );
    let text_pos = Pos2::new(
        face_rect.center().x - galley.size().x * 0.5,
        face_rect.center().y - galley.size().y * 0.5,
    );
    painter.galley_with_override_text_color(text_pos, galley, color);
}

pub fn paint_folder_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let width = 13.0;
    let height = 10.0;
    let left = center.x - width * 0.5;
    let top = center.y - height * 0.5 + 0.5;
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(left, top), Pos2::new(left + 6.0, top + 3.5)),
        CornerRadius {
            nw: 2,
            ne: 1,
            sw: 0,
            se: 0,
        },
        color,
    );
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(left, top + 2.5),
            Pos2::new(left + width, top + height),
        ),
        CornerRadius::same(2),
        color,
    );
}

pub fn transport_icon_button(
    ui: &mut Ui,
    size: Vec2,
    filled: bool,
    tooltip: &str,
    paint: impl FnOnce(&egui::Painter, Rect, Color32),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let active = response.is_pointer_button_down_on();
    let radius = CornerRadius::same(8);
    let bg = if filled {
        if active {
            ACCENT.gamma_multiply(0.82)
        } else if hovered {
            Color32::from_rgb(214, 255, 110)
        } else {
            ACCENT
        }
    } else if active {
        Color32::from_rgb(36, 40, 50)
    } else if hovered {
        CARD_HOVER
    } else {
        CARD
    };
    let stroke = if filled {
        bg
    } else if hovered || active {
        ACCENT.gamma_multiply(0.7)
    } else {
        LINE
    };
    ui.painter().rect_filled(rect, radius, bg);
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    paint(ui.painter(), rect, if filled { INK } else { ACCENT });
    response.on_hover_text(tooltip)
}

pub fn paint_play_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(center.x - 6.0, center.y - 11.0),
            Pos2::new(center.x - 6.0, center.y + 11.0),
            Pos2::new(center.x + 12.0, center.y),
        ],
        color,
        Stroke::NONE,
    ));
}

pub fn paint_pause_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    painter.rect_filled(
        Rect::from_center_size(Pos2::new(center.x - 6.0, center.y), Vec2::new(5.0, 18.0)),
        CornerRadius::same(1),
        color,
    );
    painter.rect_filled(
        Rect::from_center_size(Pos2::new(center.x + 6.0, center.y), Vec2::new(5.0, 18.0)),
        CornerRadius::same(1),
        color,
    );
}

pub fn paint_to_start_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    painter.rect_filled(
        Rect::from_center_size(Pos2::new(center.x - 9.0, center.y), Vec2::new(3.0, 16.0)),
        CornerRadius::same(1),
        color,
    );
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(center.x + 10.0, center.y - 9.0),
            Pos2::new(center.x + 10.0, center.y + 9.0),
            Pos2::new(center.x - 5.0, center.y),
        ],
        color,
        Stroke::NONE,
    ));
}

pub fn paint_trim_handle(painter: &egui::Painter, x: f32, track: Rect, is_in: bool, active: bool) {
    let color = if active { TEXT } else { ACCENT };
    let bar = Rect::from_min_max(
        Pos2::new(x - 2.0, track.top()),
        Pos2::new(x + 2.0, track.bottom()),
    );
    painter.rect_filled(bar, CornerRadius::same(2), color);

    let cap_w = 18.0;
    let cap_h = 20.0;
    let cap = if is_in {
        Rect::from_min_max(
            Pos2::new(x - cap_w + 2.0, track.bottom() - 6.0),
            Pos2::new(x + 2.0, track.bottom() - 6.0 + cap_h),
        )
    } else {
        Rect::from_min_max(
            Pos2::new(x - 2.0, track.bottom() - 6.0),
            Pos2::new(x + cap_w - 2.0, track.bottom() - 6.0 + cap_h),
        )
    };
    painter.rect_filled(cap, CornerRadius::same(4), color);
    painter.rect_stroke(
        cap,
        CornerRadius::same(4),
        Stroke::new(1.0, INK.gamma_multiply(0.35)),
        egui::StrokeKind::Inside,
    );
    for index in 0..3 {
        let gy = cap.top() + 6.0 + index as f32 * 4.0;
        painter.line_segment(
            [
                Pos2::new(cap.left() + 4.0, gy),
                Pos2::new(cap.right() - 4.0, gy),
            ],
            Stroke::new(1.4, INK),
        );
    }
}

pub fn paint_playhead(painter: &egui::Painter, x: f32, track: Rect) {
    painter.rect_filled(
        Rect::from_center_size(
            Pos2::new(x, track.center().y),
            Vec2::new(2.0, track.height()),
        ),
        CornerRadius::same(1),
        TEXT,
    );
    let cap = Rect::from_center_size(Pos2::new(x, track.top() - 6.0), Vec2::new(14.0, 14.0));
    painter.rect_filled(cap, CornerRadius::same(3), TEXT);
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(x - 7.0, track.top() + 1.0),
            Pos2::new(x + 7.0, track.top() + 1.0),
            Pos2::new(x, track.top() + 10.0),
        ],
        TEXT,
        Stroke::NONE,
    ));
}
