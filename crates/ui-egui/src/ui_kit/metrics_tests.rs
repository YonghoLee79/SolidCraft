use crate::ui_kit::metrics::dimension;
use crate::ui_kit::test_support::*;
use crate::ui_kit::*;

use crate::scenario;

#[test]
fn non_finite_and_tiny_row_widths_are_sanitized() {
    assert_eq!(dimension(f32::NAN), 0.0);
    assert_eq!(dimension(f32::INFINITY), 0.0);
    assert_eq!(dimension(-10.0), 0.0);
    let ctx = crate::fonts::test_context();
    for width in [0.0, 1.0, 12.0] {
        frame(&ctx, vec![], |ui| {
            ui.set_width(width);
            row(ui, "Tiny", |ui| {
                let mut value = String::new();
                field(ui, &mut value);
            });
        });
        assert!(scenario::rect_of("dialog:row:Tiny").unwrap().is_finite());
    }
}

#[test]
fn file_tiles_are_fixed_height_bounded_and_disabled_actions_stay_inert() {
    let ctx = crate::fonts::test_context();
    for width in [80.0, 264.0, 680.0] {
        frame(&ctx, vec![], |ui| {
            ui.set_width(width);
            let tile = FileView::action_tile(ui, "file:new", width, "new", "New design", "Millimetres", true);
            assert_eq!(tile.rect.height(), crate::theme::Tokens::get().file_tile_height);
            assert!(tile.rect.width() <= width);
            let row = FileView::nav_row(ui, "file:save", "Save", "Ctrl+S", false, false);
            assert!(!row.enabled());
            assert!(!row.clicked());
            assert_eq!(row.rect.height(), crate::theme::Tokens::get().file_nav_height);
        });
    }
}
