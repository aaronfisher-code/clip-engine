use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame, Margin, Pos2,
    Rect, Sense, Shadow, Shape, Stroke, Ui, Vec2, Visuals,
};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(10, 11, 13);
pub const PANEL: Color32 = Color32::from_rgb(16, 18, 22);
pub const CARD: Color32 = Color32::from_rgb(22, 25, 31);
pub const CARD_HOVER: Color32 = Color32::from_rgb(28, 32, 40);
pub const LINE: Color32 = Color32::from_rgb(42, 48, 58);
pub const ACCENT: Color32 = Color32::from_rgb(232, 176, 74);
pub const ACCENT_DIM: Color32 = Color32::from_rgba_premultiplied(51, 39, 16, 56);
pub const TEXT: Color32 = Color32::from_rgb(236, 233, 226);
pub const MUTED: Color32 = Color32::from_rgb(148, 154, 164);
pub const DANGER: Color32 = Color32::from_rgb(232, 112, 104);
pub const OK: Color32 = Color32::from_rgb(110, 186, 122);
pub const INK: Color32 = Color32::from_rgb(18, 16, 12);

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

    let mut style = (*ctx.style()).clone();
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(20.0, medium()),
    );
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
    style.visuals = ctx.style().visuals.clone();
    ctx.set_style(style);
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
    Frame::NONE
        .fill(BG)
        .inner_margin(Margin::symmetric(16, 14))
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

pub fn progress_bar(ui: &mut Ui, progress: f32) {
    let height = 10.0;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        Sense::hover(),
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(5), Color32::from_rgb(14, 16, 20));
    ui.painter()
        .rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    let filled = rect.width() * progress.clamp(0.0, 1.0);
    if filled > 0.0 {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(filled.max(6.0).min(rect.width()), rect.height())),
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
    ui.painter()
        .circle_filled(center, 26.0, Color32::from_rgba_unmultiplied(12, 13, 16, 210));
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
            Color32::from_rgb(210, 154, 58)
        } else if hovered {
            Color32::from_rgb(242, 190, 92)
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

pub fn paint_trim_handle(
    painter: &egui::Painter,
    x: f32,
    track: Rect,
    is_in: bool,
    active: bool,
) {
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
            [Pos2::new(cap.left() + 4.0, gy), Pos2::new(cap.right() - 4.0, gy)],
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
    let cap = Rect::from_center_size(
        Pos2::new(x, track.top() - 6.0),
        Vec2::new(14.0, 14.0),
    );
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
