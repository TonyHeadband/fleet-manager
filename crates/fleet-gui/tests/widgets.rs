use eframe::egui;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{default_ui_schema, Agent, AgentId, FieldDef, FieldKind, Status, Tone, Workspace};
use fleet_gui::theme::{palette, tone_color};
use fleet_gui::widgets::{badge, button, field, tabs, toggle, ButtonVariant};
use std::cell::RefCell;
use std::rc::Rc;

fn ws(repo: &str) -> Workspace {
    Workspace { repo: repo.into(), branch: "fleet/fix-the-tests".into(), worktree: format!("/w/{repo}"), base: "abc".into(), has_changes: false }
}

fn agent() -> Agent {
    Agent {
        id: AgentId("a1".into()),
        task: "do it".into(),
        workdir: "/w".into(),
        workspaces: vec![ws("core")],
        tmux_session: "fleet-a1".into(),
        status: Status::Working,
        activity: "running Bash".into(),
        status_since: 1000,
        has_changes: false,
        paused: false,
        owner: "antonio".into(),
        blocked_reason: None,
    }
}

fn def(key: &str, label: &str, kind: FieldKind) -> FieldDef {
    FieldDef { key: key.into(), label: label.into(), kind, show_when: fleet_api::Condition::Always }
}

#[test]
fn button_click_is_reported_and_disabled_buttons_do_not_click() {
    let clicks = Rc::new(RefCell::new((0, 0)));
    let c = clicks.clone();
    let mut h = Harness::new_ui(move |ui| {
        if button(ui, "Go", ButtonVariant::Default).clicked() {
            c.borrow_mut().0 += 1;
        }
        ui.add_enabled_ui(false, |ui| {
            if button(ui, "Off", ButtonVariant::Danger).clicked() {
                c.borrow_mut().1 += 1;
            }
        });
    });
    h.get_by_label("Go").click();
    h.run();
    h.get_by_label("Off").click();
    h.run();
    assert_eq!(*clicks.borrow(), (1, 0));
}

#[test]
fn every_button_variant_draws_its_label() {
    let h = Harness::new_ui(|ui| {
        for (l, v) in [("Plain", ButtonVariant::Default), ("Confirm", ButtonVariant::Danger), ("Kill", ButtonVariant::QuietDanger)] {
            button(ui, l, v);
        }
    });
    for l in ["Plain", "Confirm", "Kill"] {
        h.get_by_label(l);
    }
}

#[test]
fn toggle_shows_pressed_state() {
    let h = Harness::new_ui(|ui| {
        toggle(ui, "On", true);
        toggle(ui, "Off", false);
    });
    assert_eq!(h.get_by_label("On").accesskit_node().toggled(), Some(Toggled::True));
    assert_eq!(h.get_by_label("Off").accesskit_node().toggled(), Some(Toggled::False));
}

#[test]
fn badge_shows_text_with_label_as_tooltip() {
    let mut h = Harness::new_ui(|ui| {
        ui.style_mut().interaction.tooltip_delay = 0.0;
        badge(ui, "core", "Repos");
    });
    h.get_by_label("core");
    h.get_by_label("core").hover();
    h.run();
    h.get_by_label("Repos");
}

#[test]
fn tabs_mark_the_selected_tab_and_report_a_click_once() {
    let picked = Rc::new(RefCell::new(Vec::new()));
    let p = picked.clone();
    let mut h = Harness::new_ui(move |ui| {
        if let Some(id) = tabs(ui, &[("a", "Alpha"), ("b", "Beta")], "a") {
            p.borrow_mut().push(id);
        }
    });
    assert_eq!(h.get_by_label("Alpha").accesskit_node().toggled(), Some(Toggled::True));
    assert_eq!(h.get_by_label("Beta").accesskit_node().toggled(), Some(Toggled::False));
    h.get_by_label("Alpha").click();
    h.run();
    assert!(picked.borrow().is_empty(), "clicking the selected tab is not a change");
    h.get_by_label("Beta").click();
    h.run();
    assert_eq!(*picked.borrow(), ["b"]);
}

#[test]
fn field_renders_each_kind() {
    let a = agent();
    let h = Harness::new_ui(move |ui| {
        field(ui, &def("task", "Task", FieldKind::Text), &a, 1300);
        field(ui, &def("branch", "Branch", FieldKind::Mono), &a, 1300);
        field(ui, &def("status_since", "In status", FieldKind::Duration), &a, 1300);
        field(ui, &def("activity", "Activity", FieldKind::Badge), &a, 1300);
    });
    h.get_by_label("do it");
    h.get_by_label("fleet/fix-the-tests");
    h.get_by_label("5m");
    h.get_by_label("running Bash");
}

#[test]
fn field_renders_nothing_for_empty_false_or_unknown() {
    let mut a = agent();
    a.activity = String::new();
    let h = Harness::new_ui(move |ui| {
        for key in ["activity", "paused", "blocked_reason", "no_such_field"] {
            field(ui, &def(key, "X", FieldKind::Badge), &a, 1);
        }
        ui.label("end");
    });
    h.get_by_label("end");
    assert!(h.query_by_label("X").is_none());
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::Label).count(), 1, "only the sentinel label exists");
}

#[test]
fn field_shows_a_true_flag_as_its_label_and_a_list_as_one_badge_per_entry() {
    let mut a = agent();
    a.paused = true;
    a.workspaces = vec![ws("core"), ws("portal")];
    let h = Harness::new_ui(move |ui| {
        field(ui, &def("paused", "Paused", FieldKind::Badge), &a, 1);
        field(ui, &def("repos", "Repos", FieldKind::Badge), &a, 1);
    });
    h.get_by_label("Paused");
    h.get_by_label("core");
    h.get_by_label("portal");
}

/// The hex colours the web UI uses, read from its stylesheet so the two front ends cannot drift apart.
fn web_tokens() -> (Vec<(String, String)>, Vec<(String, String)>) {
    let css = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/src/styles.css")).unwrap();
    let mut found: Vec<(String, String)> = Vec::new();
    for line in css.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("--tone-") {
            if let Some((name, value)) = rest.split_once(':') {
                found.push((name.to_string(), value.trim().trim_end_matches(';').to_string()));
            }
        }
    }
    assert_eq!(found.len(), 10, "five tones in :root and five in the dark block: {found:?}");
    (found[..5].to_vec(), found[5..].to_vec())
}

fn hex(c: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

#[test]
fn tone_colors_match_the_web_tokens() {
    let (light, dark) = web_tokens();
    let tone = |name: &str| match name {
        "neutral" => Tone::Neutral,
        "active" => Tone::Active,
        "attention" => Tone::Attention,
        "done" => Tone::Done,
        "muted" => Tone::Muted,
        other => panic!("unknown tone {other}"),
    };
    for (set, is_dark) in [(light, false), (dark, true)] {
        for (name, value) in set {
            assert_eq!(hex(tone_color(tone(&name), is_dark)), value.to_lowercase(), "{name} dark={is_dark}");
        }
    }
}

#[test]
fn the_palette_has_distinct_light_and_dark_surfaces_and_the_danger_colour() {
    let (l, d) = (palette(false), palette(true));
    assert_ne!(l.bg, d.bg);
    assert_ne!(l.text, d.text);
    assert_eq!(hex(l.danger), "#b42318");
    assert_eq!(hex(d.danger), "#ff7b72");
    let _ = default_ui_schema();
}
