//! File/start presentation; callers retain commands, caches and recovery ownership.
use super::{metrics::dimension, text::paint_text};
use crate::theme::Tokens;
use egui::{Align2, FontId, InnerResponse, Rect, Response, Sense, Stroke, TextureId, Ui, Vec2, pos2, vec2};

pub(crate) struct RecentRow<'a> {
    pub title: &'a str,
    pub path: &'a str,
    pub when: &'a str,
    pub workspace: &'a str,
    pub thumbnail: Option<(TextureId, Vec2)>,
}

pub(crate) struct SampleCard<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub icon: &'a str,
    pub thumbnail: Option<(TextureId, Vec2)>,
}

/// Shared start-page surfaces. Every allocation is bounded by the caller's width.
pub(crate) struct FileView;

impl FileView {
    pub(crate) fn title(ui: &mut Ui, title: &str) {
        let t = Tokens::get();
        let galley = ui.painter().layout_no_wrap(title.to_owned(), crate::fonts::semibold(t.font_start_title), t.form_ink);
        let (rect, _) = ui.allocate_exact_size(vec2(dimension(ui.available_width()), galley.size().y), Sense::hover());
        ui.painter().with_clip_rect(ui.clip_rect().intersect(rect)).galley(rect.min, galley, t.form_ink);
    }

    pub(crate) fn section_title(ui: &mut Ui, title: &str) {
        let t = Tokens::get();
        let galley = ui.painter().layout_no_wrap(title.to_owned(), crate::fonts::semibold(t.font_intro), t.form_ink);
        let (rect, _) = ui.allocate_exact_size(vec2(dimension(ui.available_width()), galley.size().y), Sense::hover());
        ui.painter().with_clip_rect(ui.clip_rect().intersect(rect)).galley(rect.min, galley, t.form_ink);
    }

    pub(crate) fn tile_min_width() -> f32 {
        Tokens::get().form_card_width - Tokens::get().space_24
    }
    pub(crate) fn sample_min_width() -> f32 {
        Tokens::get().file_nav_width - Tokens::get().file_tile_height
    }

    pub(crate) fn navigation<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let t = Tokens::get();
        egui::Frame::NONE.fill(t.file_rail).inner_margin(egui::Margin::symmetric(t.space_10 as i8, t.space_14 as i8)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = t.space_2;
            add(ui)
        })
    }

    pub(crate) fn nav_row(ui: &mut Ui, id: &str, label: &str, shortcut: &str, selected: bool, enabled: bool) -> Response {
        let t = Tokens::get();
        ui.add_enabled_ui(enabled, |ui| {
            let (rect, response) = allocate(ui, id, ui.available_width(), t.file_nav_height, label);
            if selected || response.hovered() || response.has_focus() {
                ui.painter().rect_filled(rect, t.radius_lg, t.file_rail_hover);
            }
            let inner = rect.shrink2(vec2(t.space_12, 0.0));
            let key_width =
                ui.painter().layout_no_wrap(shortcut.to_owned(), FontId::monospace(t.font_small), t.file_rail_ink).size().x.min(inner.width());
            let label_rect = Rect::from_min_max(inner.min, pos2((inner.right() - key_width - t.space_8).max(inner.left()), inner.bottom()));
            let ink = if ui.is_enabled() { t.file_rail_ink } else { t.file_rail_ink.gamma_multiply(0.5) };
            paint_text(
                ui,
                label_rect,
                label,
                if selected { crate::fonts::semibold(t.font_heading) } else { FontId::proportional(t.font_heading) },
                ink,
                false,
            );
            paint_text(
                ui,
                Rect::from_min_max(pos2(inner.right() - key_width, inner.top()), inner.max),
                shortcut,
                FontId::monospace(t.font_small),
                ink,
                false,
            );
            response
        })
        .inner
    }

    pub(crate) fn back(ui: &mut Ui, id: &str, label: &str, shortcut: &str) -> Response {
        let response = Self::nav_row(ui, id, label, shortcut, false, true);
        ui.add_space(Tokens::get().space_10);
        response
    }

    pub(crate) fn separator(ui: &mut Ui) {
        let t = Tokens::get();
        ui.add_space(t.space_10);
        let (rect, _) = ui.allocate_exact_size(vec2(dimension(ui.available_width()), 1.0), Sense::hover());
        ui.painter().hline(
            (rect.left() + t.space_12)..=(rect.right() - t.space_12).max(rect.left() + t.space_12),
            rect.center().y,
            Stroke::new(1.0, t.file_rail_ink.gamma_multiply(0.25)),
        );
        ui.add_space(t.space_10);
    }

    pub(crate) fn action_tile(ui: &mut Ui, id: &str, width: f32, icon: &str, label: &str, subtitle: &str, primary: bool) -> Response {
        let t = Tokens::get();
        let (rect, response) = allocate(ui, id, width, t.file_tile_height, label);
        card(ui, rect, &response, primary);
        let inner = rect.shrink(t.space_16);
        let icon_size = t.file_tile_icon.min(inner.height()).min(inner.width());
        let icon_rect = Rect::from_center_size(pos2(inner.left() + icon_size / 2.0, inner.center().y), Vec2::splat(icon_size));
        crate::icons::paint_accented(ui.painter(), icon_rect, icon, None, if ui.is_enabled() { 1.0 } else { 0.45 });
        let text_rect = Rect::from_min_max(pos2((icon_rect.right() + t.space_14).min(inner.right()), inner.top()), inner.max);
        let title_bottom = (text_rect.top() + t.font_heading + t.space_4).min(text_rect.bottom());
        let title = Rect::from_min_max(text_rect.min, pos2(text_rect.right(), title_bottom));
        let ink = if primary { t.file_primary_ink } else { t.form_ink };
        paint_text(ui, title, label, crate::fonts::semibold(t.font_heading), ink, false);
        let caption = Rect::from_min_max(pos2(text_rect.left(), title_bottom), text_rect.max);
        let mut job = egui::text::LayoutJob::simple(subtitle.to_owned(), FontId::proportional(t.font_label), t.form_dim, caption.width());
        job.wrap.max_rows = 2;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        ui.painter().with_clip_rect(ui.clip_rect().intersect(caption)).galley(caption.min, galley, t.form_dim);
        response.on_hover_text(format!("{label}\n{subtitle}"))
    }

    pub(crate) fn recent_row(ui: &mut Ui, id: &str, row: RecentRow<'_>) -> Response {
        let t = Tokens::get();
        let (rect, response) = allocate(ui, id, ui.available_width(), t.file_recent_height, row.title);
        ui.painter().rect_filled(rect, 0, if response.hovered() || response.has_focus() { t.hover } else { t.file_surface });
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.file_border));
        let inner = rect.shrink2(vec2(t.space_14, t.space_8));
        let preview = Rect::from_min_size(inner.min, vec2(t.space_24 * 2.0, inner.height()).min(inner.size()));
        thumbnail(ui, preview, row.thumbnail);
        if row.thumbnail.is_none() {
            crate::icons::paint_accented(
                ui.painter(),
                Rect::from_center_size(preview.center(), Vec2::splat(t.space_24.min(preview.height()))),
                "box",
                None,
                1.0,
            );
        }
        let left = (preview.right() + t.space_14).min(inner.right());
        let content = Rect::from_min_max(pos2(left, inner.top()), inner.max);
        // At narrow widths the same row keeps title and path, leaving optional metadata out.
        if content.width() < t.form_card_width * 2.0 {
            let half = content.center().y;
            paint_text(
                ui,
                Rect::from_min_max(content.min, pos2(content.right(), half)),
                row.title,
                crate::fonts::semibold(t.font_button),
                t.form_ink,
                false,
            );
            paint_text(ui, Rect::from_min_max(pos2(left, half), content.max), row.path, FontId::monospace(t.font_body), t.form_dim, false);
        } else {
            let when_width = t.space_24 * 5.0;
            let workspace_width = t.form_label_width * 2.0;
            let remaining = (content.width() - when_width - workspace_width - t.space_14 * 3.0).max(0.0);
            let title_width = remaining * 0.4;
            let path_left = left + title_width + t.space_14;
            let when_left = content.right() - workspace_width - t.space_14 - when_width;
            let workspace_left = content.right() - workspace_width;
            paint_text(
                ui,
                Rect::from_min_max(content.min, pos2(left + title_width, content.bottom())),
                row.title,
                crate::fonts::semibold(t.font_button),
                t.form_ink,
                false,
            );
            paint_text(
                ui,
                Rect::from_min_max(pos2(path_left, content.top()), pos2((when_left - t.space_14).max(path_left), content.bottom())),
                row.path,
                FontId::monospace(t.font_body),
                t.form_dim,
                false,
            );
            paint_text(
                ui,
                Rect::from_min_max(pos2(when_left, content.top()), pos2(when_left + when_width, content.bottom())),
                row.when,
                FontId::proportional(t.font_label),
                t.form_dim,
                false,
            );
            paint_text(
                ui,
                Rect::from_min_max(pos2(workspace_left, content.top()), content.max),
                row.workspace,
                FontId::proportional(t.font_label),
                t.form_dim,
                false,
            );
        }
        response.on_hover_text(row.path)
    }

    pub(crate) fn sample_card(ui: &mut Ui, id: &str, width: f32, sample: SampleCard<'_>) -> Response {
        let t = Tokens::get();
        let line_height = ui.fonts_mut(|fonts| fonts.row_height(&FontId::proportional(t.font_body)));
        let height = t.file_sample_image_height + t.space_10 * 2.0 + t.control_height + t.space_8 + line_height * 3.0;
        let (rect, response) = allocate(ui, id, width, height, sample.title);
        card(ui, rect, &response, false);
        let inner = rect.shrink(t.space_10);
        let image = Rect::from_min_size(inner.min, vec2(inner.width(), t.file_sample_image_height));
        thumbnail(ui, image, sample.thumbnail);
        if sample.thumbnail.is_none() {
            crate::icons::paint_accented(
                ui.painter(),
                Rect::from_center_size(image.center(), Vec2::splat(t.file_tile_icon.min(image.width()))),
                sample.icon,
                None,
                1.0,
            );
        }
        let title = Rect::from_min_size(pos2(inner.left(), image.bottom() + t.space_8), vec2(inner.width(), t.control_height));
        paint_text(ui, title, sample.title, crate::fonts::semibold(t.font_button), t.form_ink, false);
        let mut job = egui::text::LayoutJob::simple(sample.description.to_owned(), FontId::proportional(t.font_body), t.form_dim, inner.width());
        job.wrap.max_rows = 3;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        ui.painter().with_clip_rect(ui.clip_rect().intersect(inner)).galley(pos2(inner.left(), title.bottom()), galley, t.form_dim);
        response.on_hover_text(sample.description)
    }

    pub(crate) fn recovery<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
        let t = Tokens::get();
        egui::Frame::NONE
            .fill(t.file_recovery)
            .stroke(Stroke::new(1.0, t.file_recovery_border))
            .corner_radius(t.radius_xl)
            .inner_margin(egui::Margin::symmetric(t.space_16 as i8, t.space_12 as i8))
            .show(ui, |ui| {
                ui.visuals_mut().override_text_color = Some(t.file_recovery_ink);
                add(ui)
            })
    }
}

fn allocate(ui: &mut Ui, id: &str, width: f32, height: f32, label: &str) -> (Rect, Response) {
    let (rect, _) = ui.allocate_exact_size(vec2(dimension(width).min(dimension(ui.available_width())), height), Sense::hover());
    let response = ui.interact(rect, ui.make_persistent_id(id), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    if response.clicked() {
        response.request_focus();
    }
    if response.enabled() && response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (rect, response)
}

fn card(ui: &Ui, rect: Rect, response: &Response, primary: bool) {
    let t = Tokens::get();
    let fill = if primary {
        t.file_primary
    } else if response.hovered() || response.has_focus() {
        t.hover
    } else {
        t.file_surface
    };
    let border = if primary || response.has_focus() { t.file_primary_border } else { t.file_border };
    ui.painter().rect(rect, t.radius_xl, fill, Stroke::new(if primary { 1.5 } else { 1.0 }, border), egui::StrokeKind::Inside);
}

fn thumbnail(ui: &Ui, rect: Rect, texture: Option<(TextureId, Vec2)>) {
    let t = Tokens::get();
    ui.painter().rect(rect, t.radius_md, t.file_surface, Stroke::new(1.0, t.file_border), egui::StrokeKind::Inside);
    if let Some((texture, size)) = texture.filter(|(_, size)| size.is_finite() && size.x > 0.0 && size.y > 0.0) {
        let scale = (rect.width() / size.x).min(rect.height() / size.y);
        let target = Align2::CENTER_CENTER.align_size_within_rect(size * scale, rect);
        ui.painter().image(texture, target, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), t.file_image_tint);
    }
}
