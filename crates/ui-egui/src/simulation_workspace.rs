//! The Simulation workspace: flow studies on the design's bodies. So far it is the frame for
//! them: the workspace, its SIMULATION tab and each study's setup dialog (`dialogs_sim`). The
//! view stays the modelling viewport, so a study's body is picked as in Design. The solvers, and
//! the study setup saved beside the design, come later.

use crate::SolveApp;
use crate::dialogs::Kind;
use crate::workspace_switch::Workspace;

pub fn active(app: &SolveApp) -> bool {
    crate::workspace_switch::current(app) == Workspace::Simulation
}

/// Leaving Simulation closes a study dialog left open; a Design dialog is left alone.
pub(crate) fn close(app: &mut SolveApp) {
    if app.dialog.as_ref().is_some_and(|d| matches!(d.kind, Kind::Sim(_))) {
        crate::dialogs::cancel(app);
    }
}

/// Another document in the window: it opens in Design.
pub fn reset_document(app: &mut SolveApp) {
    if active(app) {
        close(app);
        app.ui.tab = Workspace::Design.tab().into();
    }
}
