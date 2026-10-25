use eframe::egui;
use egui::accesskit::Role;
use egui::accesskit::Toggled;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use fleet_api::{Agent, AgentId, FieldDef, FieldKind, Status, Tone, Workspace};
use fleet_gui::theme::{self, tokens, tone_color, Weight};
use fleet_gui::widgets::{
    badge, button, dot, field, format_bytes, icon_button, tabs, toggle, BadgeTone, ButtonVariant,
    Icon,
};
use std::cell::RefCell;
use std::rc::Rc;

fn ws(repo: &str) -> Workspace {
    Workspace {
        repo: repo.into(),
        branch: "fleet/fix-the-tests".into(),
        worktree: format!("/w/{repo}"),
        base: "abc".into(),
        from_branch: None,
        has_changes: false,
        owned: false,
    }
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
        repo_request: None,
        stop_cause: None,
        disk_bytes: None,
    }
}

fn def(key: &str, label: &str, kind: FieldKind) -> FieldDef {
    FieldDef {
        key: key.into(),
        label: label.into(),
        kind,
        show_when: fleet_api::Condition::Always,
        value_labels: Default::default(),
        value_tones: Default::default(),
    }
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
        for (l, v) in [
            ("Primary", ButtonVariant::Primary),
            ("Plain", ButtonVariant::Default),
            ("Ghost", ButtonVariant::Ghost),
            ("Confirm", ButtonVariant::Danger),
            ("Kill", ButtonVariant::QuietDanger),
        ] {
            button(ui, l, v);
        }
    });
    for l in ["Primary", "Plain", "Ghost", "Confirm", "Kill"] {
        h.get_by_label(l);
    }
}

#[test]
fn toggle_shows_pressed_state() {
    let h = Harness::new_ui(|ui| {
        toggle(ui, "On", true);
        toggle(ui, "Off", false);
    });
    assert_eq!(
        h.get_by_label("On").accesskit_node().toggled(),
        Some(Toggled::True)
    );
    assert_eq!(
        h.get_by_label("Off").accesskit_node().toggled(),
        Some(Toggled::False)
    );
}

#[test]
fn badge_shows_text_with_label_as_tooltip() {
    let mut h = Harness::new_ui(|ui| {
        ui.style_mut().interaction.tooltip_delay = 0.0;
        badge(ui, "core", "Repos", BadgeTone::Neutral);
    });
    h.get_by_label("core");
    h.get_by_label("core").hover();
    h.run();
    h.get_by_label("Repos");
}

fn rect_fills(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Color32> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
        match shape {
            egui::Shape::Rect(r) => out.push(r.fill),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|s| walk(&s.shape, &mut out));
    out
}

/// What the button's frame actually paints at rest, hovered and while pressed.
fn painted_fills(variant: ButtonVariant) -> [Vec<egui::Color32>; 3] {
    let mut h = Harness::new_ui(move |ui| {
        button(ui, "Go", variant);
    });
    h.run();
    let rest = rect_fills(&h.output().shapes);
    h.get_by_label("Go").hover();
    h.run();
    let hovered = rect_fills(&h.output().shapes);
    h.event(egui::Event::PointerButton {
        pos: h.get_by_label("Go").rect().center(),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    h.run();
    let pressed = rect_fills(&h.output().shapes);
    [rest, hovered, pressed]
}

#[test]
fn primary_and_danger_answer_hover_and_press_with_their_own_fills() {
    // Harness uses egui's default (dark) theme.
    let t = tokens(true);
    for (variant, resting) in [
        (ButtonVariant::Primary, t.fill),
        (ButtonVariant::Danger, t.danger),
    ] {
        let [rest, hovered, pressed] = painted_fills(variant);
        assert!(rest.contains(&resting), "{variant:?} rests on its fill");
        assert_ne!(hovered, rest, "{variant:?} must change fill on hover");
        assert_ne!(
            pressed, hovered,
            "{variant:?} must change fill again while pressed"
        );
    }
}

#[test]
fn a_disabled_primary_button_keeps_its_resting_fill() {
    let mut h = Harness::new_ui(|ui| {
        ui.add_enabled_ui(false, |ui| button(ui, "Go", ButtonVariant::Primary));
    });
    h.run();
    // egui fades a disabled widget by scaling the alpha of everything it paints.
    let faded = tokens(true)
        .fill
        .gamma_multiply(egui::Visuals::dark().disabled_alpha());
    assert!(rect_fills(&h.output().shapes).contains(&faded));
}

#[test]
fn icon_button_is_a_button_named_by_its_label() {
    let clicks = Rc::new(RefCell::new(0));
    let c = clicks.clone();
    let mut h = Harness::new_ui(move |ui| {
        if icon_button(ui, Icon::Close, "Close").clicked() {
            *c.borrow_mut() += 1;
        }
    });
    h.get_by_role_and_label(Role::Button, "Close").click();
    h.run();
    assert_eq!(*clicks.borrow(), 1);
}

#[test]
fn dot_allocates_a_square_of_its_diameter() {
    let size = Rc::new(RefCell::new(egui::Vec2::ZERO));
    let s = size.clone();
    let _h = Harness::new_ui(move |ui| {
        *s.borrow_mut() = dot(ui, egui::Color32::RED, 7.0).rect.size();
    });
    assert_eq!(*size.borrow(), egui::vec2(7.0, 7.0));
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
    assert_eq!(
        h.get_by_label("Alpha").accesskit_node().toggled(),
        Some(Toggled::True)
    );
    assert_eq!(
        h.get_by_label("Beta").accesskit_node().toggled(),
        Some(Toggled::False)
    );
    h.get_by_label("Alpha").click();
    h.run();
    assert!(
        picked.borrow().is_empty(),
        "clicking the selected tab is not a change"
    );
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
        field(
            ui,
            &def("status_since", "In status", FieldKind::Duration),
            &a,
            1300,
        );
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
    assert_eq!(
        h.query_all_by_role(egui::accesskit::Role::Label).count(),
        1,
        "only the sentinel label exists"
    );
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

fn hex(c: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

#[test]
fn tokens_match_the_spec_table() {
    // (bg, surface, raised, border, text_high, text, text_low, fill, on_fill, danger)
    let dark = [
        "#1c1c1c", "#212121", "#292929", "#333333", "#f5f5f5", "#c4c4c4", "#8f8f8f", "#f5f5f5",
        "#141414", "#e5534b",
    ];
    let light = [
        "#f2f2f2", "#ffffff", "#e3e3e3", "#d9d9d9", "#0d0d0d", "#333333", "#636363", "#141414",
        "#ffffff", "#c4302b",
    ];
    for (is_dark, want) in [(true, dark), (false, light)] {
        let t = tokens(is_dark);
        let got = [
            t.bg,
            t.surface,
            t.raised,
            t.border,
            t.text_high,
            t.text,
            t.text_low,
            t.fill,
            t.on_fill,
            t.danger,
        ]
        .map(hex);
        assert_eq!(got, want, "dark={is_dark}");
    }
}

#[test]
fn tone_colors_match_the_spec() {
    // (tone, light, dark)
    let spec = [
        (Tone::Neutral, "#8a94a6", "#8a94a6"),
        (Tone::Active, "#2f6fed", "#6b9bff"),
        (Tone::Attention, "#d97706", "#f5a524"),
        (Tone::Done, "#15803d", "#4ade80"),
        (Tone::Muted, "#7c5cc4", "#b39ddb"),
        (Tone::Caution, "#ca8a04", "#facc15"),
        (Tone::Danger, "#c4302b", "#e5534b"),
    ];
    for (tone, light, dark) in spec {
        assert_eq!(hex(tone_color(tone, false)), light, "{tone:?} light");
        assert_eq!(hex(tone_color(tone, true)), dark, "{tone:?} dark");
    }
}

#[test]
fn light_and_dark_differ_where_they_must() {
    let (l, d) = (tokens(false), tokens(true));
    assert_ne!(l.bg, d.bg);
    assert_ne!(l.surface, d.surface);
    assert_ne!(l.text_high, d.text_high);
    assert_ne!(l.fill, d.fill);
}

#[test]
fn apply_installs_plex_first_and_keeps_egui_fonts_as_fallback() {
    let ctx = egui::Context::default();
    let egui_defaults = egui::FontDefinitions::default();
    theme::apply(&ctx);
    // Fonts install at the start of a frame; the texture deltas must be cleared or dropping them panics.
    for _ in 0..2 {
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
    }

    assert!(ctx
        .fonts(|f| f.families())
        .contains(&egui::FontFamily::Name(theme::MEDIUM_FAMILY.into())));
    let defs = ctx.fonts(|f| f.definitions().clone());
    for (family, plex_key) in [
        (egui::FontFamily::Proportional, "plex-sans"),
        (egui::FontFamily::Monospace, "plex-mono"),
    ] {
        let keys = &defs.families[&family];
        assert_eq!(keys[0], plex_key, "{family:?}");
        for default_key in &egui_defaults.families[&family] {
            assert!(keys.contains(default_key), "{family:?} lost {default_key}");
        }
    }
}

fn applied_visuals(dark: bool) -> egui::Visuals {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    ctx.style_of(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    })
    .visuals
    .clone()
}

/// WCAG relative luminance, from the gamma-encoded channels.
fn luminance(c: egui::Color32) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `top` (premultiplied, possibly translucent) painted over an opaque `under`.
fn over(top: egui::Color32, under: egui::Color32) -> egui::Color32 {
    let keep = 1.0 - top.a() as f32 / 255.0;
    let mix = |t: u8, u: u8| (t as f32 + u as f32 * keep).round().min(255.0) as u8;
    egui::Color32::from_rgb(
        mix(top.r(), under.r()),
        mix(top.g(), under.g()),
        mix(top.b(), under.b()),
    )
}

#[test]
fn selected_text_is_clearly_visible_in_both_themes() {
    for dark in [true, false] {
        let t = tokens(dark);
        let sel = applied_visuals(dark).selection.bg_fill;
        // Terminal and diff text sit on `bg`, the prompt and reply boxes on `bg` inside `surface`.
        for under in [t.bg, t.surface] {
            let shown = over(sel, under);
            assert!(
                contrast(shown, under) >= 1.5,
                "dark={dark}: selection {shown:?} on {under:?} is {:.2}:1",
                contrast(shown, under)
            );
            assert!(
                contrast(t.text, shown) >= 4.5,
                "dark={dark}: text on the selection is {:.2}:1",
                contrast(t.text, shown)
            );
        }
    }
}

#[test]
fn the_text_cursor_is_a_neutral_two_pixel_caret() {
    for dark in [true, false] {
        assert_eq!(
            applied_visuals(dark).text_cursor.stroke,
            egui::Stroke::new(2.0, tokens(dark).text_high),
            "dark={dark}"
        );
    }
}

#[test]
fn medium_falls_back_without_apply() {
    let mut h = Harness::new_ui(|ui| {
        assert_eq!(
            theme::sans(ui, 12.0, Weight::Medium).family,
            egui::FontFamily::Proportional
        );
    });
    h.run();
}

fn set_focus(h: &mut Harness<'_>, focused: bool) {
    h.input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .focused = Some(focused);
}

#[test]
fn animate_is_off_without_focus() {
    let seen = Rc::new(RefCell::new((true, 0.0f32)));
    let s = seen.clone();
    let mut h = Harness::new_ui(move |ui| {
        *s.borrow_mut() = (theme::animate(ui.ctx()), theme::pulse(ui, 1.6));
    });
    h.run();
    assert_eq!(*seen.borrow(), (false, 1.0));
}

#[test]
fn animate_and_pulse_follow_focus() {
    let seen = Rc::new(RefCell::new((false, 0.0f32)));
    let s = seen.clone();
    let mut h = Harness::new_ui(move |ui| {
        *s.borrow_mut() = (theme::animate(ui.ctx()), theme::pulse(ui, 1.6));
    });
    // `step`, not `run`: a focused pulse schedules a repaint, which would make `run` loop.
    set_focus(&mut h, true);
    h.input_mut().time = Some(10.0);
    h.step();
    let (on, first) = *seen.borrow();
    assert!(on);
    assert!((0.35..=1.0).contains(&first), "pulse {first}");

    h.input_mut().time = Some(10.4);
    h.step();
    let (_, second) = *seen.borrow();
    assert!((0.35..=1.0).contains(&second), "pulse {second}");
    assert!(
        (first - second).abs() > 0.05,
        "pulse did not move: {first} then {second}"
    );

    set_focus(&mut h, false);
    h.step();
    assert_eq!(*seen.borrow(), (false, 1.0));
}

#[test]
fn badge_field_shows_the_schema_label_for_a_raw_value_and_falls_back_to_the_raw_text() {
    let mut a = agent();
    a.blocked_reason = Some(fleet_api::BlockedReason::RepoAccess);
    a.activity = "unlabelled".into();
    let labelled = FieldDef {
        value_labels: [("repo_access".to_string(), "Repo access".to_string())].into(),
        ..def("blocked_reason", "Blocked", FieldKind::Badge)
    };
    let h = Harness::new_ui(move |ui| {
        field(ui, &labelled, &a, 1);
        field(ui, &def("activity", "Activity", FieldKind::Badge), &a, 1);
    });
    h.get_by_label("Repo access");
    h.get_by_label("unlabelled");
    assert!(h.query_by_label("repo_access").is_none());
}

#[test]
fn format_bytes_uses_si_units_with_one_decimal() {
    for (bytes, want) in [
        (0, "0 B"),
        (999, "999 B"),
        (12_000, "12.0 KB"),
        (1_500_000, "1.5 MB"),
        (7_600_000_000, "7.6 GB"),
        (2_340_000_000_000, "2.3 TB"),
    ] {
        assert_eq!(format_bytes(bytes), want, "{bytes}");
    }
}

#[test]
fn a_bytes_field_shows_the_size() {
    let mut a = agent();
    a.disk_bytes = Some(12_000);
    let h = Harness::new_ui(move |ui| {
        field(ui, &def("disk_bytes", "Size", FieldKind::Bytes), &a, 1);
    });
    h.get_by_label("12.0 KB");
}

// ---- path elision ----

fn at_most(n: usize) -> impl Fn(&str) -> bool {
    move |s: &str| s.chars().count() <= n
}

#[test]
fn elide_middle_leaves_a_path_that_fits() {
    assert_eq!(
        fleet_gui::widgets::elide_middle("/tmp/a/b", &at_most(8)),
        "/tmp/a/b"
    );
}

#[test]
fn elide_middle_keeps_the_root_and_the_last_component() {
    let out = fleet_gui::widgets::elide_middle("/tmp/fb-fp.bi0kg9/home/dev/foo-feat", &at_most(21));
    assert!(out.chars().count() <= 21, "{out}");
    assert!(out.starts_with("/tmp"), "{out}");
    assert!(out.ends_with("/foo-feat"), "{out}");
    assert!(out.contains('…'), "{out}");
    let sibling = fleet_gui::widgets::elide_middle("/tmp/fb-fp.bi0kg9/home/dev/foo", &at_most(21));
    assert_ne!(out, sibling);
}

#[test]
fn elide_middle_cuts_a_huge_last_component_from_its_front() {
    let out = fleet_gui::widgets::elide_middle("/a/ééééééééééééééééééééééé-tail", &at_most(12));
    assert!(out.chars().count() <= 12, "{out}");
    assert!(out.ends_with("-tail"), "{out}");
    let out = fleet_gui::widgets::elide_middle("ééééééééééééééééééé", &at_most(5));
    assert!(out.chars().count() <= 5 && out.starts_with('…'), "{out}");
}

#[test]
fn elide_middle_survives_a_budget_nothing_fits() {
    let _ = fleet_gui::widgets::elide_middle("/ééé/ééé", &at_most(0));
    let _ = fleet_gui::widgets::elide_middle("", &at_most(0));
}

#[test]
fn a_path_field_not_drawn_during_the_retry_window_does_not_take_focus_later() {
    use fleet_gui::widgets::path_edit;
    struct P {
        text: String,
        draw: bool,
        tail: bool,
    }
    let mut h = Harness::builder().build_ui_state(
        |ui, s: &mut P| {
            if s.draw {
                let id = ui.make_persistent_id("p");
                path_edit(ui, id, &mut s.text, 200.0, None, "", s.tail);
            }
        },
        P {
            text: "/some/path".into(),
            draw: true,
            tail: true,
        },
    );
    h.step();
    h.state_mut().draw = false;
    for _ in 0..12 {
        h.step();
    }
    h.state_mut().draw = true;
    h.state_mut().tail = false;
    h.run();
    assert!(
        !h.get_by_role(egui::accesskit::Role::TextInput)
            .accesskit_node()
            .is_focused(),
        "a stale pick took focus"
    );
}
