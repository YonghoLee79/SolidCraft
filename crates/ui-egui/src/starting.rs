//! The window while the app gets ready (#1210). The window stays hidden until its first frame is
//! drawn, so the setup that is slow on some computers (fonts, the 3D view's shaders, the recovery
//! scan, a design to open) runs in later frames, one step each, behind a light "Starting" page:
//! the first frame only paints that page, and each later one runs the next step and names the one
//! after it.

use std::collections::VecDeque;

use egui::{Align2, FontId, Id, Sense, Stroke, ViewportCommand, vec2};

use crate::theme::Tokens;

/// One step of the setup, run on its own frame with what it sets up (`T`, the app).
pub type Step<T> = Box<dyn FnOnce(&mut T, &egui::Context)>;

/// The steps still to run. `T` is what they set up: the desktop app's window state.
pub struct Starting<T> {
    steps: VecDeque<(&'static str, Step<T>)>,
    /// Whether a frame showed the page yet: no step runs before it has.
    shown: bool,
    total: usize,
}

impl<T> Default for Starting<T> {
    fn default() -> Self {
        Self { steps: VecDeque::new(), shown: false, total: 0 }
    }
}

impl<T> Starting<T> {
    /// A step, after the ones already added. `label` (a message id; empty for none) says on the
    /// page what is being done while the step runs.
    #[must_use]
    pub fn then(mut self, label: &'static str, step: impl FnOnce(&mut T, &egui::Context) + 'static) -> Self {
        self.steps.push_back((label, Box::new(step)));
        self.total += 1;
        self
    }

    /// Whether every step has run.
    pub fn done(&self) -> bool {
        self.steps.is_empty()
    }

    /// The start of a frame: on the first one nothing runs (the page goes up first); on each
    /// later one, the next step. Returns whether the app is still starting, so this frame shows
    /// the page again.
    pub fn logic(&mut self, target: &mut T, ctx: &egui::Context) -> bool {
        if !self.shown {
            self.shown = true;
            ctx.request_repaint();
            return !self.done();
        }
        if let Some((_, step)) = self.steps.pop_front() {
            step(target, ctx);
        }
        if self.done() {
            return false;
        }
        ctx.request_repaint();
        true
    }

    /// The page: the name, a progress bar and what the next step does, in the theme's colours.
    /// Dragging it moves the window (there may be no OS title bar yet).
    pub fn ui(&self, ui: &mut egui::Ui) {
        let t = Tokens::get();
        let label = self.steps.front().map_or("", |(label, _)| *label);
        let done = self.total - self.steps.len();
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.panel)).show(ui, |ui| {
            let rect = ui.max_rect();
            let response = ui.interact(rect, Id::new("starting-drag"), Sense::click_and_drag());
            if response.drag_started_by(egui::PointerButton::Primary) {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }
            let painter = ui.painter();
            let c = rect.center();
            painter.text(c - vec2(0.0, 28.0), Align2::CENTER_BOTTOM, "SolidCraft", FontId::proportional(30.0), t.text);
            // Steps done of all of them; a step that runs long holds the bar where it is.
            let bar = egui::Rect::from_center_size(c, vec2(240.0, 4.0));
            painter.rect_filled(bar, 2.0, t.border);
            let share = if self.total == 0 { 1.0 } else { done as f32 / self.total as f32 };
            painter.rect_filled(egui::Rect::from_min_size(bar.min, vec2(bar.width() * share.clamp(0.0, 1.0), bar.height())), 2.0, t.accent);
            if !label.is_empty() {
                painter.text(c + vec2(0.0, 20.0), Align2::CENTER_TOP, crate::i18n::tr(label), FontId::proportional(14.0), t.text_dim);
            }
            painter.rect_stroke(rect, 0.0, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(output: &egui::FullOutput) -> Vec<String> {
        let mut out = Vec::new();
        for shape in &output.shapes {
            if let egui::Shape::Text(t) = &shape.shape {
                out.push(t.galley.text().to_string());
            }
        }
        out
    }

    fn frame(ctx: &egui::Context, starting: &mut Starting<Vec<&'static str>>, log: &mut Vec<&'static str>) -> (bool, Vec<String>) {
        let mut still = false;
        let mut output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1000.0))), ..Default::default() },
            |ui| {
                still = starting.logic(log, ui.ctx());
                if still {
                    starting.ui(ui);
                }
            },
        );
        output.textures_delta.clear();
        (still, texts(&output))
    }

    /// The first frame (the one that shows the window) is drawn before any step runs, however
    /// slow; each later frame runs one step and names the next.
    #[test]
    fn the_first_frame_shows_the_page_before_any_step_runs() {
        let ctx = egui::Context::default();
        let mut starting = Starting::default()
            .then("", |log: &mut Vec<&'static str>, _| log.push("fonts"))
            .then("starting.view", |log: &mut Vec<&'static str>, _| log.push("view"))
            .then("starting.recovery", |log: &mut Vec<&'static str>, _| log.push("recovery"));
        let mut log = Vec::new();

        let (still, painted) = frame(&ctx, &mut starting, &mut log);
        assert!(still);
        assert!(log.is_empty(), "no step ran before the first frame: {log:?}");
        assert!(painted.iter().any(|t| t == "SolidCraft"), "{painted:?}");
        assert!(ctx.has_requested_repaint(), "the next frame comes without input");

        let (still, painted) = frame(&ctx, &mut starting, &mut log);
        assert!(still);
        assert_eq!(log, ["fonts"]);
        assert!(painted.iter().any(|t| t == "Preparing the 3D view…"), "the page names the step the next frame runs: {painted:?}");

        let (still, painted) = frame(&ctx, &mut starting, &mut log);
        assert!(still);
        assert_eq!(log, ["fonts", "view"]);
        assert!(painted.iter().any(|t| t == "Checking for unsaved work…"), "{painted:?}");

        let (still, painted) = frame(&ctx, &mut starting, &mut log);
        assert!(!still, "the app draws this frame itself");
        assert!(starting.done());
        assert_eq!(log, ["fonts", "view", "recovery"]);
        assert!(painted.is_empty(), "{painted:?}");
    }

    #[test]
    fn no_steps_means_the_app_draws_from_the_second_frame() {
        let ctx = egui::Context::default();
        let mut starting = Starting::default();
        let mut log = Vec::new();
        assert!(!frame(&ctx, &mut starting, &mut log).0);
        assert!(!frame(&ctx, &mut starting, &mut log).0);
    }
}
