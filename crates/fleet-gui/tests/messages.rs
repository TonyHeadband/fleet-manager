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
