import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openApp, pending } from "../management/harness";
import { agent, fakeClient, fakeNative, RESTARTED } from "../test/fixtures";
import { UpdateNotice } from "./UpdateNotice";
import { useVersionOffer } from "./VersionOffer";

// Ported from crates/fleet-gui/tests/shell.rs (the restart offer) and tests/update_notice.rs.
function VersionOffer(props: Parameters<typeof useVersionOffer>[0]) {
  return useVersionOffer(props).dialog;
}

function offer(over: { version?: unknown; weStarted?: boolean; loaded?: boolean } = {}) {
  const client = fakeClient({ version: vi.fn().mockResolvedValue("version" in over ? over.version : { version: "0.1.0" }) });
  const native = fakeNative();
  // A restart that asks nothing and is over at once.
  const restart = { ask: vi.fn(), restarting: null, confirming: false };
  const view = render(<VersionOffer client={client} native={native} weStarted={over.weStarted ?? false} loaded={over.loaded ?? true} restart={restart} />);
  return { client, native, restart, view };
}

describe("The version offer, as egui's", () => {
  it("an_older_daemon_offers_a_restart", async () => {
    const { restart } = offer();
    expect(await screen.findByText("The background service is from v0.1.0 — restart it?")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Restart" }));
    expect(restart.ask).toHaveBeenCalledWith({ kind: "restart" });
    expect(screen.queryByText(/restart it\?/)).toBeNull();
  });

  it("a_daemon_without_a_version_route_offers_a_restart", async () => {
    offer({ version: null });
    expect(await screen.findByText("The background service is from an older version — restart it?")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Keep using it" }));
    expect(screen.queryByText(/restart it\?/)).toBeNull();
  });

  it("the_same_version_shows_nothing", async () => {
    const { client } = offer({ version: { version: "0.2.0" } });
    await waitFor(() => expect(client.version).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.queryByText(/restart it\?/)).toBeNull();
  });

  it("a_daemon_started_by_this_window_is_not_asked_about", async () => {
    const { client } = offer({ weStarted: true });
    await act(async () => {});
    expect(client.version).not.toHaveBeenCalled();
  });

  it("the_offer_waits_for_the_first_agent_snapshot", async () => {
    const { client, native, restart, view } = offer({ loaded: false });
    await act(async () => {});
    expect(client.version).not.toHaveBeenCalled();
    view.rerender(<VersionOffer client={client} native={native} weStarted={false} loaded restart={restart} />);
    expect(await screen.findByText("The background service is from v0.1.0 — restart it?")).toBeInTheDocument();
  });
});

describe("The update notice, as egui's", () => {
  afterEach(() => vi.useRealTimers());
  const release = { version: "0.3.0", notes: "Faster boards.", page: "https://example.invalid/r/0.3.0" };
  const available = { kind: "available", ...release, skipped: false };

  it("a_newer_release_shows_the_modal", async () => {
    const native = fakeNative({ checkUpdate: vi.fn().mockResolvedValue(available) });
    render(<UpdateNotice native={native} blocked={false} />);
    expect(await screen.findByRole("heading", { name: "Fleet Manager v0.3.0 is available (you have v0.2.0)" })).toBeInTheDocument();
    expect(screen.getByText("Faster boards.")).toBeInTheDocument();
    expect(screen.getByText("Download opens the release page in your browser.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Download" }));
    expect(native.openUrl).toHaveBeenCalledWith(release.page);
    expect(screen.queryByRole("heading", { name: /is available/ })).toBeNull();
  });

  it("skip_this_version_hides_it_next_time", async () => {
    const native = fakeNative({ checkUpdate: vi.fn().mockResolvedValue(available) });
    render(<UpdateNotice native={native} blocked={false} />);
    await userEvent.click(await screen.findByRole("checkbox", { name: "Skip this version" }));
    await userEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(native.skipVersion).toHaveBeenCalledWith("0.3.0");
  });

  it("a_skipped_release_shows_no_notice", async () => {
    const native = fakeNative({ checkUpdate: vi.fn().mockResolvedValue({ ...available, skipped: true }) });
    render(<UpdateNotice native={native} blocked={false} />);
    await waitFor(() => expect(native.checkUpdate).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.queryByRole("heading", { name: /is available/ })).toBeNull();
  });

  it("later_shows_the_release_again_at_the_next_check", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const native = fakeNative({ checkUpdate: vi.fn().mockResolvedValue(available) });
    render(<UpdateNotice native={native} blocked={false} />);
    await userEvent.click(await screen.findByRole("button", { name: "Later" }));
    expect(screen.queryByRole("heading", { name: /is available/ })).toBeNull();
    await act(() => vi.advanceTimersByTimeAsync(24 * 3600 * 1000));
    expect(await screen.findByRole("heading", { name: /is available/ })).toBeInTheDocument();
  });

  it("the_check_runs_again_every_day", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const native = fakeNative();
    render(<UpdateNotice native={native} blocked={false} />);
    await waitFor(() => expect(native.checkUpdate).toHaveBeenCalledTimes(1));
    await act(() => vi.advanceTimersByTimeAsync(24 * 3600 * 1000));
    expect(native.checkUpdate).toHaveBeenCalledTimes(2);
  });

  it("no_notice_during_first_run", async () => {
    // The persona setup is a dialog: the notice waits behind it like behind any other.
    const native = fakeNative({ checkUpdate: vi.fn().mockResolvedValue(available) });
    render(<UpdateNotice native={native} blocked />);
    await waitFor(() => expect(native.checkUpdate).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.queryByRole("heading", { name: /is available/ })).toBeNull();
  });

  it("the_notice_waits_for_another_dialog", async () => {
    const native = fakeNative({ checkUpdate: vi.fn().mockResolvedValue(available) });
    const view = render(<UpdateNotice native={native} blocked />);
    await act(async () => {});
    view.rerender(<UpdateNotice native={native} blocked={false} />);
    expect(await screen.findByRole("heading", { name: /is available/ })).toBeInTheDocument();
  });
});

describe("The version offer in the app, as egui's", () => {
  const available = { kind: "available", version: "0.3.0", notes: "", page: "https://example.invalid/r/0.3.0", skipped: false };
  const older = () => ({ version: vi.fn().mockResolvedValue({ version: "0.1.0" }) });
  const question = "The background service is from v0.1.0 — restart it?";

  it("the_update_notice_waits_for_the_offer", async () => {
    // egui's notice never shows over another dialog: the offer is answered first.
    const native = fakeNative({ checkUpdate: vi.fn().mockResolvedValue(available) });
    await openApp("Kanban", null, { native, over: older() });
    expect(await screen.findByText(question)).toBeInTheDocument();
    await waitFor(() => expect(native.checkUpdate).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.queryByRole("heading", { name: /is available/ })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Keep using it" }));
    expect(await screen.findByRole("heading", { name: /is available/ })).toBeInTheDocument();
  });

  it("a_restart_from_the_offer_shows_its_progress", async () => {
    const held = pending<typeof RESTARTED>();
    const native = fakeNative({ saveAndRestart: vi.fn().mockReturnValue(held.promise) });
    await openApp("Kanban", null, { native, over: older() });
    await userEvent.click(await screen.findByRole("button", { name: "Restart" }));
    expect(native.saveAndRestart).toHaveBeenCalledWith({ kind: "restart" });
    expect(screen.getByText("Restarting the daemon…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Keep using it" })).toBeDisabled();
    await act(async () => held.resolve(RESTARTED));
    expect(await within(screen.getByRole("contentinfo")).findByText("Daemon restarted")).toBeInTheDocument();
    expect(screen.queryByText(question)).toBeNull();
  });

  it("a_cancelled_confirmation_closes_the_offer", async () => {
    const native = fakeNative();
    await openApp("Kanban", null, { native, over: older(), agents: [agent({ id: "a", status: "working" })] });
    await userEvent.click(await screen.findByRole("button", { name: "Restart" }));
    await userEvent.click(within(screen.getByRole("dialog", { name: "Restart the daemon?" })).getByRole("button", { name: "Cancel" }));
    expect(native.saveAndRestart).not.toHaveBeenCalled();
    expect(screen.queryByText(question)).toBeNull();
  });
});
