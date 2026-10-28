use eframe::egui::text::CCursor;
use eframe::egui::text_selection::CCursorRange;
use eframe::egui::{Color32, FontId, Id, Label, Response, RichText, TextEdit, TextWrapMode, Ui};

/// Characters of the start kept when a path is elided: enough to tell `/tmp` from `/home`.
const HEAD_MIN: usize = 4;
/// Passes a field keeps retrying to take focus after a pick: the modal that returned it holds focus for two.
const TAIL_WINDOW: u64 = 4;

/// `text` cut in the middle until `fits` accepts it: the start of the path, `…`, then as many whole trailing
/// components as fit (`/tmp/fb…/dev/foo-feat`). A last component too long on its own is cut from its front.
/// `fits` is monotonic in length; it is a closure so the maths needs no font metrics.
pub fn elide_middle(text: &str, fits: &dyn Fn(&str) -> bool) -> String {
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let join = |head: usize, tail: usize| -> String {
        chars[..head]
            .iter()
            .chain(std::iter::once(&'…'))
            .chain(&chars[tail..])
            .collect()
    };
    // Tails start at a separator (`/`, on Windows `\` too; never the leading one), longest first; the longest tail
    // that still leaves room for the minimum head wins, and the head then grows into whatever space is left.
    let slashes: Vec<usize> = chars
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, c)| std::path::is_separator(**c))
        .map(|(i, _)| i)
        .collect();
    for min_head in [HEAD_MIN, 0] {
        for &tail in &slashes {
            let lo = min_head.min(tail);
            if !fits(&join(lo, tail)) {
                continue;
            }
            let (mut lo, mut hi) = (lo, tail);
            while lo < hi {
                let mid = (lo + hi).div_ceil(2);
                if fits(&join(mid, tail)) {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            return join(lo, tail);
        }
    }
    // One component wider than the space: its end is what differs, so keep that.
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if fits(&join(0, mid)) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    join(0, lo.min(chars.len()))
}

/// A path on one line in the width left. When it does not fit, the middle is elided so the last component stays
/// readable, and the full path is the hover text.
pub fn path_label(ui: &mut Ui, path: &str, font: FontId, colour: Color32) -> Response {
    prefixed_path_label(ui, "", path, font, colour)
}

/// `path_label` with a fixed lead-in (`Select: `) drawn in the same label: only the path part is elided.
pub fn prefixed_path_label(
    ui: &mut Ui,
    prefix: &str,
    path: &str,
    font: FontId,
    colour: Color32,
) -> Response {
    let width = |s: &str| {
        ui.painter()
            .layout_no_wrap(format!("{prefix}{s}"), font.clone(), colour)
            .size()
            .x
    };
    let available = ui.available_width();
    let fits = |s: &str| width(s) <= available;
    let shown = elide_middle(path, &fits);
    let elided = shown != path;
    let response = ui.add(
        Label::new(
            RichText::new(format!("{prefix}{shown}"))
                .font(font)
                .color(colour),
        )
        .wrap_mode(TextWrapMode::Extend),
    );
    if elided {
        response.on_hover_text(path)
    } else {
        response
    }
}

/// A single-line field holding a path. `show_tail` (true on the frame a path was written into it) puts the cursor
/// after the last character and focuses the field: egui only scrolls a field to its cursor while it has focus, and
/// the end of a path is the part that tells it apart. The focus is taken a frame or two later: the Browse…
/// modal is still on screen in the frame that returns the pick, and egui keeps focus inside it.
/// A non-empty field shows its whole text on hover.
pub fn path_edit(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    width: f32,
    font: Option<FontId>,
    hint: &str,
    show_tail: bool,
) -> Response {
    let ctx = ui.ctx().clone();
    let pending = id.with("show_tail");
    let now = ctx.cumulative_pass_nr();
    if show_tail {
        ctx.data_mut(|d| d.insert_temp(pending, now));
        ctx.request_repaint();
    } else if let Some(since) = ctx.data(|d| d.get_temp::<u64>(pending)) {
        // Measured in passes, not draws: a field that was not drawn during the window (the Grant box went away)
        // must not grab focus when it shows up again later.
        if now.saturating_sub(since) > TAIL_WINDOW || ctx.memory(|m| m.has_focus(id)) {
            ctx.data_mut(|d| d.remove_temp::<u64>(pending));
        } else {
            let mut state = TextEdit::load_state(&ctx, id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(text.chars().count()))));
            state.store(&ctx, id);
            ctx.memory_mut(|m| m.request_focus(id));
            ctx.request_repaint();
        }
    }
    let mut edit = TextEdit::singleline(text)
        .id(id)
        .desired_width(width)
        .hint_text(hint);
    if let Some(font) = font {
        edit = edit.font(font);
    }
    let response = ui.add(edit);
    if text.is_empty() {
        response
    } else {
        response.on_hover_text(text.as_str())
    }
}
