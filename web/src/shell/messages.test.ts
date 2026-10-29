import { describe, expect, it } from "vitest";
import { HISTORY, Messages, type Message, type Notice } from "./messages";

const msg = (notice: Notice, text: string): Message => ({ notice, text, time: 0 });
const currentText = (m: Messages) => { const c = m.current(); return c ? [c[0].text, c[1]] : null; };

// Ported one for one from crates/fleet-gui/tests/messages.rs; the footer tests are in shell.test.tsx.
describe("Messages", () => {
  it("info_shows_once_then_dims", () => {
    const m = new Messages(true);
    m.push(msg("info", "saved"), 0);
    expect(currentText(m)).toEqual(["saved", false]);
    m.tick(4.1);
    expect(currentText(m)).toEqual(["saved", true]);
  });

  it("cycles_in_arrival_order", () => {
    const m = new Messages(true);
    m.push(msg("success", "A"), 0);
    m.push(msg("info", "B"), 0);
    expect(currentText(m)).toEqual(["A", false]);
    expect(m.position()).toEqual([1, 2]);
    m.tick(4.1);
    expect(currentText(m)).toEqual(["B", false]);
    expect(m.position()).toEqual(null);
  });

  it("the_position_follows_the_cycle", () => {
    const m = new Messages(true);
    m.push(msg("error", "E1"), 0);
    m.push(msg("error", "E2"), 0);
    expect(m.position()).toEqual([1, 2]);
    m.tick(8.1);
    expect(currentText(m)).toEqual(["E2", false]);
    expect(m.position()).toEqual([2, 2]);
    m.tick(16.2);
    expect(currentText(m)).toEqual(["E1", false]);
    expect(m.position()).toEqual([1, 2]);
  });

  it("warnings_show_for_four_seconds_like_info", () => {
    const m = new Messages(true);
    m.push(msg("warning", "W"), 0);
    m.push(msg("info", "I"), 0);
    m.tick(3.9);
    expect(currentText(m)).toEqual(["W", false]);
    m.tick(4.1);
    expect(currentText(m)).toEqual(["I", false]);
  });

  it("errors_show_for_eight_seconds", () => {
    const m = new Messages(true);
    m.push(msg("error", "E"), 0);
    m.push(msg("info", "I"), 0);
    m.tick(7.9);
    expect(currentText(m)).toEqual(["E", false]);
    m.tick(8.1);
    expect(currentText(m)).toEqual(["I", false]);
  });

  it("sticky_error_loops_until_clicked", () => {
    const m = new Messages(true);
    m.push(msg("error", "E"), 0);
    m.push(msg("info", "I"), 0);
    expect(currentText(m)).toEqual(["E", false]);
    m.tick(8.1);
    expect(currentText(m)).toEqual(["I", false]);
    m.tick(12.2);
    expect(currentText(m)).toEqual(["E", false]);
    m.click(12.3);
    expect(currentText(m), "a dismissed message is never the dimmed one").toEqual(["I", true]);
    expect(m.position()).toEqual(null);
  });

  it("a_lone_sticky_error_stays_and_a_click_clears_it", () => {
    const m = new Messages(true);
    m.push(msg("warning", "W"), 0);
    m.tick(100);
    expect(currentText(m)).toEqual(["W", false]);
    expect(m.nextChange(100), "nothing to change, so no repaint to ask for").toEqual(null);
    m.click(101);
    expect(currentText(m)).toEqual(null);
  });

  it("not_sticky_error_shows_once", () => {
    const m = new Messages(false);
    m.push(msg("error", "E"), 0);
    m.push(msg("info", "I"), 0);
    m.tick(8.1);
    expect(currentText(m)).toEqual(["I", false]);
    m.tick(12.2);
    expect(currentText(m)).toEqual(["I", true]);
  });

  it("clicking_a_one_shot_message_skips_it", () => {
    const m = new Messages(true);
    m.push(msg("info", "A"), 0);
    m.push(msg("info", "B"), 0);
    m.click(1);
    expect(currentText(m)).toEqual(["B", false]);
  });

  it("duplicates_are_merged", () => {
    const m = new Messages(true);
    m.push(msg("error", "E"), 0);
    m.push(msg("error", "E"), 1);
    expect(m.position(), "the second E joined the queue").toEqual(null);
    m.push(msg("info", "I"), 1);
    m.push(msg("info", "I"), 1);
    expect(m.position()).toEqual([1, 2]);
    expect(m.history().length).toEqual(2);
  });

  it("a_new_message_replaces_the_dimmed_one", () => {
    const m = new Messages(true);
    m.push(msg("info", "A"), 0);
    m.tick(4.1);
    m.push(msg("info", "A"), 5);
    expect(currentText(m)).toEqual(["A", false]);
  });

  it("history_keeps_ten_newest_first", () => {
    const m = new Messages(true);
    for (let i = 0; i < 12; i++) {
        m.push(msg("info", `m${i}`), 0);
    }
    expect(m.history().length).toEqual(HISTORY);
    expect(m.history()[0]!.text).toEqual("m11");
    expect(m.history()[9]!.text).toEqual("m2");
  });

  it("dismissing_a_lone_loop_does_not_bring_back_an_old_message", () => {
    const m = new Messages(true);
    m.push(msg("info", "old"), 0);
    m.tick(4.1);
    expect(currentText(m)).toEqual(["old", true]);
    m.push(msg("error", "E"), 10);
    m.click(11);
    expect(currentText(m), "the dimmed leftover came from before the error: it is stale").toEqual(null);
  });

  it("a_newcomer_takes_over_at_once_from_a_lone_loop_that_had_its_turn", () => {
    const m = new Messages(true);
    m.push(msg("error", "E"), 0);
    // The window slept: no tick re-showed E, which has been up for far longer than its turn.
    m.push(msg("info", "I"), 100);
    expect(currentText(m)).toEqual(["I", false]);
    expect(m.position()).toEqual([2, 2]);
  });

  it("a_newcomer_waits_only_for_the_rest_of_a_lone_loops_turn", () => {
    const m = new Messages(true);
    m.push(msg("error", "E"), 0);
    m.push(msg("info", "I"), 5);
    expect(currentText(m)).toEqual(["E", false]);
    m.tick(8.1);
    expect(currentText(m)).toEqual(["I", false]);
  });

  it("the_position_stays_on_the_message_when_the_cycle_grows", () => {
    const m = new Messages(true);
    m.push(msg("error", "E1"), 0);
    m.push(msg("error", "E2"), 0);
    m.tick(8.1);
    expect(currentText(m)).toEqual(["E2", false]);
    expect(m.position()).toEqual([2, 2]);
    m.push(msg("error", "E3"), 9);
    expect(m.position(), "E2 is still the second").toEqual([2, 3]);
    m.tick(16.2);
    expect(currentText(m)).toEqual(["E3", false]);
    expect(m.position()).toEqual([3, 3]);
    m.tick(24.3);
    expect(currentText(m)).toEqual(["E1", false]);
    expect(m.position()).toEqual([1, 3]);
  });

  it("the_position_stays_when_a_one_shot_leaves_the_cycle", () => {
    const m = new Messages(true);
    m.push(msg("error", "E1"), 0);
    m.push(msg("info", "I"), 0);
    m.push(msg("error", "E2"), 0);
    m.tick(8.1);
    expect([currentText(m), m.position()]).toEqual([["I", false], [2, 3]]);
    m.tick(12.2);
    expect([currentText(m), m.position()]).toEqual([["E2", false], [2, 2]]);
    m.tick(20.3);
    expect([currentText(m), m.position()]).toEqual([["E1", false], [1, 2]]);
  });

});
