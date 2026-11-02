import { $, browser, expect } from "@wdio/globals";

// The Supervisor page's Storage widget, in the real webview (WebKitGTK): the bar must have been measured and drawn.
// The daemon has no task to weigh (the demo cannot start one), so the task list it answers is given two.
describe("Storage widget on Tauri", () => {
  it("draws the bar", async () => {
    await expect($("h1=Filters")).toBeDisplayed({ wait: 30_000 });
    await browser.execute(() => {
      const real = window.fetch.bind(window);
      const task = (id: string, bytes: number, state: string) => ({
        id, product: "pie", release: null, name: id, goal: id, title: null, workdir: `/x/${id}`, repos: [], state,
        stop_cause: state === "stopped" ? "stopped" : null, created_at: 1, stopped_at: null, archived_at: null, agents_used: 1,
        times: { queued: 0, working: 0, needs_you: 0, review: 0 }, log: [], last_screen: null, archive: null,
        persona: "pastry-chef", kind: "orchestrator", breakdowns: [], prs: [], done_at: null, done_by_hand: false,
        footprint: { measured_at: 1, total: bytes, items: [{ label: "core worktree", path: "/x", bytes: bytes / 2, build_bytes: null }, { label: "build output", path: "/y", bytes: bytes / 2, build_bytes: null }] },
      });
      window.fetch = async (input, init) => {
        const res = await real(input, init);
        if (!String(input).endsWith("/api/tasks")) return res;
        const body = await res.json();
        body.tasks = [task("big", 4e9, "stopped"), task("small", 2e8, "running")];
        return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
      };
      window.location.hash = "#supervisor";
    });
    await expect($('section[data-widget="storage"]')).toBeDisplayed({ wait: 15_000 });
    await browser.pause(12_000);
    const probe = await browser.execute(() => {
      const box = (el: Element | null) => (el ? { w: el.getBoundingClientRect().width, h: el.getBoundingClientRect().height } : null);
      const q = (s: string) => document.querySelector(s);
      return {
        svg: box(q('section[data-widget="storage"] svg')),
        text: (q('section[data-widget="storage"]') as HTMLElement | null)?.innerText,
      };
    });
    console.log("STORAGE PROBE", JSON.stringify(probe));
    await expect($('section[data-widget="storage"] svg')).toBeDisplayed();
    expect(probe.svg?.w ?? 0).toBeGreaterThan(0);
    // The route's numbers (the daemon's own files at least) and the clean-up list with its Archive buttons.
    expect(probe.text).toContain("daemon files");
    expect(probe.text).toMatch(/archive/i);
  });
});
