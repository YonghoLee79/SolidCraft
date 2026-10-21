//! The only constructors for outer UI hosts. Content widgets belong to the other kit modules.
//! A surface declares its purpose; the shell supplies the layout once per frame. Never infer
//! floating/docked presentation from a panel name or keep a second layout preference here.
use crate::frame::Side;
use crate::{shell::LayoutMode, theme::Tokens};
use egui::{Color32, Context, Frame, Id, Stroke, WidgetText};

/// Persistent panels adapt to the layout; transient UI retains its anchoring and modality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// Browser, History, Render settings and future workspace panels.
    Panel,
    /// Feature forms and the Assistant sheet.
    Feature,
    /// Sketch options, including their translucent background.
    Palette,
    /// Workspace switcher and ribbon cards.
    Chrome,
    /// Movable system windows and modal dialogs; never become workspace docks.
    Window,
    /// Anchored menus, completion lists and rename editors.
    Popup,
    /// Model-anchored editors and manipulation feedback.
    Canvas,
    /// Status/progress notifications, independent of the workspace layout.
    Toast,
    /// Viewport navigation controls (the compass itself is drawn geometry).
    Navigation,
    /// Full-page content, the model viewport and native pop-out content.
    Viewport,
    /// Deliberate input-blocking backdrops; must not inherit CAD input blocking.
    InputShield,
}

#[derive(Clone, Copy, Debug)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// Universal outer host and surface policy. Both host geometry and frame construction must go
/// through this API, including code in nested modules and platform apps.
#[derive(Clone, Copy)]
pub struct Container {
    surface: Surface,
    layout: LayoutMode,
}

impl Container {
    pub fn new(ctx: &Context, surface: Surface) -> Self {
        Self::for_layout(LayoutMode::current(ctx), surface)
    }

    pub(crate) fn for_layout(layout: LayoutMode, surface: Surface) -> Self {
        Self { surface, layout }
    }

    /// Shell-owned placement policy, shared by current and future adaptive panels.
    pub fn floating(self) -> bool {
        matches!(self.surface, Surface::Panel | Surface::Feature | Surface::Palette | Surface::Chrome) && self.layout == LayoutMode::Floating
    }

    /// Tokenised outer chrome. Callers may supply content padding, not another card surface.
    pub fn frame(self, ctx: &Context, fill: Color32, side: crate::frame::Side) -> Frame {
        let t = Tokens::get();
        let flat = Frame::NONE.fill(fill);
        match self.surface {
            Surface::Feature | Surface::Palette if !self.floating() => dock_frame(ctx, side).fill(fill).inner_margin(0.0),
            Surface::Panel | Surface::Chrome if !self.floating() => flat.stroke(egui::Stroke::new(1.0, t.border)),
            Surface::Feature | Surface::Window => {
                flat.stroke(egui::Stroke::new(1.0, t.dialog_border)).corner_radius(t.radius_xl).shadow(t.shell_shadow())
            }
            Surface::Palette => flat.stroke(egui::Stroke::new(1.0, t.border)).corner_radius(t.radius_xl).shadow(t.shell_shadow()),
            Surface::Panel | Surface::Chrome => flat.stroke(egui::Stroke::new(1.0, t.border)).corner_radius(t.radius_lg).shadow(t.shell_shadow()),
            Surface::Navigation => flat.stroke(egui::Stroke::new(1.0, t.border)).corner_radius(t.radius_md),
            Surface::Toast => flat.stroke(egui::Stroke::new(1.0, t.border)).corner_radius(t.radius_md).shadow(t.shell_shadow()),
            Surface::Popup => Self::popup_frame(&ctx.global_style()).fill(fill),
            Surface::Canvas | Surface::Viewport | Surface::InputShield => flat,
        }
    }

    /// Surface of an allocated dock or internal pinned band. Floating card decoration is
    /// supplied by `frame`; edge-hosted panes must not acquire outer shadows or card gaps.
    pub fn edge_frame(self, fill: Color32) -> Frame {
        Frame::NONE.fill(fill)
    }

    pub fn area(self, id: Id) -> egui::Area {
        egui::Area::new(id).enabled(self.input_enabled())
    }

    pub fn edge(self, id: impl Into<Id>, edge: Edge) -> egui::Panel {
        match edge {
            Edge::Left => egui::Panel::left(id),
            Edge::Right => egui::Panel::right(id),
            Edge::Top => egui::Panel::top(id),
            Edge::Bottom => egui::Panel::bottom(id),
        }
    }

    pub(super) fn window<'a>(self, title: impl Into<WidgetText>) -> egui::Window<'a> {
        egui::Window::new(title).enabled(self.input_enabled())
    }

    /// Anchored editors inherit their containing widget's local style. Their host stays a
    /// popup in both layouts; only the universal module constructs the egui popup frame.
    pub fn popup_frame(style: &egui::Style) -> Frame {
        Frame::popup(style)
    }

    pub(crate) fn menu_frame() -> Frame {
        let t = Tokens::get();
        egui::Frame::NONE
            .fill(t.dialog_body)
            .stroke(Stroke::new(1.0, t.dialog_border))
            .corner_radius(t.radius_xl)
            .inner_margin(egui::Margin::symmetric(t.space_6 as i8, t.space_6 as i8))
            .shadow(egui::Shadow {
                offset: [0, t.space_14 as i8],
                blur: (t.space_24 + t.space_12) as u8,
                spread: 0,
                color: t.app_bar.gamma_multiply(0.18),
            })
    }

    pub(crate) fn submenu(self) -> egui::menu::SubMenu {
        egui::menu::SubMenu::new()
    }

    pub fn menu<'a>(self, response: &egui::Response) -> egui::Popup<'a> {
        egui::Popup::menu(response)
    }

    pub fn context_menu<'a>(self, response: &egui::Response) -> egui::Popup<'a> {
        egui::Popup::context_menu(response)
    }

    pub(crate) fn tooltip_frame(self, ctx: &Context) -> Frame {
        let t = Tokens::get();
        self.frame(ctx, t.dialog_body, crate::frame::Side::Right)
            .stroke(egui::Stroke::new(1.0, t.dialog_border))
            .inner_margin(egui::Margin::symmetric(t.space_8 as i8, t.space_4 as i8))
    }

    pub fn modal(self, id: Id) -> egui::Modal {
        egui::Modal::new(id).area(egui::Modal::default_area(id).fade_in(false))
    }

    pub fn central(self) -> egui::CentralPanel {
        egui::CentralPanel::default()
    }

    fn input_enabled(self) -> bool {
        self.surface == Surface::InputShield || !crate::issue_report::cad_input_blocked()
    }
}

/// The window frame of a docked panel: the two corners away from its edge are rounded, and the
/// shadow falls the same way, so a left dock mirrors a right one.
fn dock_frame(ctx: &egui::Context, side: Side) -> egui::Frame {
    let t = crate::theme::Tokens::get();
    let (corner_radius, shadow_x) = match side {
        Side::Right => (egui::CornerRadius { nw: t.radius_sm, sw: t.radius_sm, ne: 0, se: 0 }, -2),
        Side::Left => (egui::CornerRadius { nw: 0, sw: 0, ne: t.radius_sm, se: t.radius_sm }, 2),
    };
    egui::Frame::window(&ctx.global_style())
        .fill(t.dialog_bg)
        .stroke(egui::Stroke::new(1.0, t.dialog_border))
        .corner_radius(corner_radius)
        .shadow(egui::Shadow { offset: [shadow_x, 2], blur: 8, spread: 0, color: t.shell_shadow().color })
}
