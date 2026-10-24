use fleet_gui::messages::{Message, Messages, HISTORY};
use fleet_log::Notice;

fn msg(notice: Notice, text: &str) -> Message {
    Message { notice, text: text.into(), time: 0 }
}

fn current_text(m: &Messages) -> Option<(&str, bool)> {
    m.current().map(|(m, dimmed)| (m.text.as_str(), dimmed))
}

#[test]
fn info_shows_once_then_dims() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Info, "saved"), 0.0);
    assert_eq!(current_text(&m), Some(("saved", false)));
    m.tick(4.1);
    assert_eq!(current_text(&m), Some(("saved", true)));
}

#[test]
fn cycles_in_arrival_order() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Success, "A"), 0.0);
    m.push(msg(Notice::Info, "B"), 0.0);
    assert_eq!(current_text(&m), Some(("A", false)));
    assert_eq!(m.position(), Some((1, 2)));
    m.tick(4.1);
    assert_eq!(current_text(&m), Some(("B", false)));
    assert_eq!(m.position(), None);
}

#[test]
fn the_position_follows_the_cycle() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E1"), 0.0);
    m.push(msg(Notice::Error, "E2"), 0.0);
    assert_eq!(m.position(), Some((1, 2)));
    m.tick(8.1);
    assert_eq!(current_text(&m), Some(("E2", false)));
    assert_eq!(m.position(), Some((2, 2)));
    m.tick(16.2);
    assert_eq!(current_text(&m), Some(("E1", false)));
    assert_eq!(m.position(), Some((1, 2)));
}

#[test]
fn warnings_show_for_four_seconds_like_info() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Warning, "W"), 0.0);
    m.push(msg(Notice::Info, "I"), 0.0);
    m.tick(3.9);
    assert_eq!(current_text(&m), Some(("W", false)));
    m.tick(4.1);
    assert_eq!(current_text(&m), Some(("I", false)));
}

#[test]
fn errors_show_for_eight_seconds() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E"), 0.0);
    m.push(msg(Notice::Info, "I"), 0.0);
    m.tick(7.9);
    assert_eq!(current_text(&m), Some(("E", false)));
    m.tick(8.1);
    assert_eq!(current_text(&m), Some(("I", false)));
}

#[test]
fn sticky_error_loops_until_clicked() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E"), 0.0);
    m.push(msg(Notice::Info, "I"), 0.0);
    assert_eq!(current_text(&m), Some(("E", false)));
    m.tick(8.1);
    assert_eq!(current_text(&m), Some(("I", false)));
    m.tick(12.2);
    assert_eq!(current_text(&m), Some(("E", false)));
    m.click(12.3);
    assert_eq!(current_text(&m), Some(("I", true)), "a dismissed message is never the dimmed one");
    assert_eq!(m.position(), None);
}

#[test]
fn a_lone_sticky_error_stays_and_a_click_clears_it() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Warning, "W"), 0.0);
    m.tick(100.0);
    assert_eq!(current_text(&m), Some(("W", false)));
    assert_eq!(m.next_change(100.0), None, "nothing to change, so no repaint to ask for");
    m.click(101.0);
    assert_eq!(current_text(&m), None);
}

#[test]
fn not_sticky_error_shows_once() {
    let mut m = Messages::new(false);
    m.push(msg(Notice::Error, "E"), 0.0);
    m.push(msg(Notice::Info, "I"), 0.0);
    m.tick(8.1);
    assert_eq!(current_text(&m), Some(("I", false)));
    m.tick(12.2);
    assert_eq!(current_text(&m), Some(("I", true)));
}

#[test]
fn clicking_a_one_shot_message_skips_it() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Info, "A"), 0.0);
    m.push(msg(Notice::Info, "B"), 0.0);
    m.click(1.0);
    assert_eq!(current_text(&m), Some(("B", false)));
}

#[test]
fn duplicates_are_merged() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E"), 0.0);
    m.push(msg(Notice::Error, "E"), 1.0);
    assert_eq!(m.position(), None, "the second E joined the queue");
    m.push(msg(Notice::Info, "I"), 1.0);
    m.push(msg(Notice::Info, "I"), 1.0);
    assert_eq!(m.position(), Some((1, 2)));
    assert_eq!(m.history().len(), 2);
}

#[test]
fn a_new_message_replaces_the_dimmed_one() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Info, "A"), 0.0);
    m.tick(4.1);
    m.push(msg(Notice::Info, "A"), 5.0);
    assert_eq!(current_text(&m), Some(("A", false)));
}

#[test]
fn history_keeps_ten_newest_first() {
    let mut m = Messages::new(true);
    for i in 0..12 {
        m.push(msg(Notice::Info, &format!("m{i}")), 0.0);
    }
    assert_eq!(m.history().len(), HISTORY);
    assert_eq!(m.history()[0].text, "m11");
    assert_eq!(m.history()[9].text, "m2");
}

/// The footer draws `i/n`, not a fixed `1/n`: found on the real window, where the second of two looping errors
/// still read `1/2`.
#[test]
fn the_footer_counter_shows_the_message_on_screen() {
    use eframe::egui;
    use egui_kittest::kittest::Queryable;
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E1"), 0.0);
    m.push(msg(Notice::Error, "E2"), 0.0);
    // E1 holds the screen for an error's turn (`SHOW_ERROR`, 8 s), so 8.1 is just after E2 takes over.
    let state = (m, 8.1);
    let mut h = egui_kittest::Harness::builder().with_size(egui::Vec2::new(800.0, 40.0)).build_ui_state(
        |ui, (m, now): &mut (Messages, f64)| {
            fleet_gui::components::footer(ui, m, *now);
        },
        state,
    );
    h.run();
    h.get_by_label("E2");
    h.get_by_label("2/2");
}

#[test]
fn dismissing_a_lone_loop_does_not_bring_back_an_old_message() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Info, "old"), 0.0);
    m.tick(4.1);
    assert_eq!(current_text(&m), Some(("old", true)));
    m.push(msg(Notice::Error, "E"), 10.0);
    m.click(11.0);
    assert_eq!(current_text(&m), None, "the dimmed leftover came from before the error: it is stale");
}

#[test]
fn a_newcomer_takes_over_at_once_from_a_lone_loop_that_had_its_turn() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E"), 0.0);
    // The window slept: no tick re-showed E, which has been up for far longer than its turn.
    m.push(msg(Notice::Info, "I"), 100.0);
    assert_eq!(current_text(&m), Some(("I", false)));
    assert_eq!(m.position(), Some((2, 2)));
}

#[test]
fn a_newcomer_waits_only_for_the_rest_of_a_lone_loops_turn() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E"), 0.0);
    m.push(msg(Notice::Info, "I"), 5.0);
    assert_eq!(current_text(&m), Some(("E", false)));
    m.tick(8.1);
    assert_eq!(current_text(&m), Some(("I", false)));
}

#[test]
fn the_position_stays_on_the_message_when_the_cycle_grows() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E1"), 0.0);
    m.push(msg(Notice::Error, "E2"), 0.0);
    m.tick(8.1);
    assert_eq!(current_text(&m), Some(("E2", false)));
    assert_eq!(m.position(), Some((2, 2)));
    m.push(msg(Notice::Error, "E3"), 9.0);
    assert_eq!(m.position(), Some((2, 3)), "E2 is still the second");
    m.tick(16.2);
    assert_eq!(current_text(&m), Some(("E3", false)));
    assert_eq!(m.position(), Some((3, 3)));
    m.tick(24.3);
    assert_eq!(current_text(&m), Some(("E1", false)));
    assert_eq!(m.position(), Some((1, 3)));
}

#[test]
fn the_position_stays_when_a_one_shot_leaves_the_cycle() {
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "E1"), 0.0);
    m.push(msg(Notice::Info, "I"), 0.0);
    m.push(msg(Notice::Error, "E2"), 0.0);
    m.tick(8.1);
    assert_eq!((current_text(&m), m.position()), (Some(("I", false)), Some((2, 3))));
    m.tick(12.2);
    assert_eq!((current_text(&m), m.position()), (Some(("E2", false)), Some((2, 2))));
    m.tick(20.3);
    assert_eq!((current_text(&m), m.position()), (Some(("E1", false)), Some((1, 2))));
}

fn footer_harness(m: Messages, now: f64) -> egui_kittest::Harness<'static, (Messages, f64)> {
    use eframe::egui;
    let mut h = egui_kittest::Harness::builder().with_size(egui::Vec2::new(800.0, 40.0)).build_ui_state(
        |ui, (m, now): &mut (Messages, f64)| {
            ui.style_mut().interaction.tooltip_delay = 0.0;
            fleet_gui::components::footer(ui, m, *now);
        },
        (m, now),
    );
    h.run();
    h
}

#[test]
fn hovering_the_footer_shows_the_whole_message() {
    use egui_kittest::kittest::Queryable;
    let mut m = Messages::new(true);
    m.push(msg(Notice::Error, "first line\nsecond line"), 0.0);
    let mut h = footer_harness(m, 0.0);
    h.get_by_label("first line second line").hover();
    h.run();
    h.get_by_label("first line\nsecond line");
}

#[test]
fn the_footer_points_only_when_a_click_does_something() {
    use eframe::egui::{CursorIcon, Event};
    use egui_kittest::kittest::Queryable;
    let mut m = Messages::new(true);
    m.push(msg(Notice::Info, "saved"), 0.0);
    let mut h = footer_harness(m, 0.0);
    let at = h.get_by_label("saved").rect().center();
    h.event(Event::PointerMoved(at));
    h.run();
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::PointingHand, "a click skips it");
    h.state_mut().1 = 4.1;
    h.run();
    assert_eq!(h.state().0.current().map(|(_, dimmed)| dimmed), Some(true));
    h.event(Event::PointerMoved(at));
    h.run();
    assert_ne!(h.output().platform_output.cursor_icon, CursorIcon::PointingHand, "a dimmed message ignores clicks");
}
