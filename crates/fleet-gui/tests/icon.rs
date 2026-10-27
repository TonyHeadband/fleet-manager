use eframe::egui::{self, ViewportCommand, ViewportId};
use fleet_gui::icon::{self, IconSync};

#[test]
fn pngs_decode_at_256() {
    let dark = eframe::icon_data::from_png_bytes(icon::png(true)).unwrap();
    let light = eframe::icon_data::from_png_bytes(icon::png(false)).unwrap();
    assert_eq!((dark.width, dark.height), (256, 256));
    assert_eq!((light.width, light.height), (256, 256));
    assert_ne!(dark.rgba, light.rgba);
}

fn frame(ctx: &egui::Context, sync: &mut IconSync, dark: bool) -> (bool, usize) {
    let mut sent = false;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        sent = sync.update(ui.ctx(), dark);
    });
    // Nothing renders these frames; egui asserts that texture deltas are consumed.
    out.textures_delta.clear();
    let icons = out.viewport_output[&ViewportId::ROOT]
        .commands
        .iter()
        .filter(|c| matches!(c, ViewportCommand::Icon(Some(_))))
        .count();
    (sent, icons)
}

#[test]
fn the_icon_is_sent_once_per_change() {
    let ctx = egui::Context::default();
    let mut sync = IconSync::default();
    assert_eq!(frame(&ctx, &mut sync, true), (true, 1));
    for _ in 0..3 {
        assert_eq!(frame(&ctx, &mut sync, true), (false, 0));
    }
    assert_eq!(frame(&ctx, &mut sync, false), (true, 1));
}
