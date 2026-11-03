import { $, $$, browser, expect } from "@wdio/globals";

// The Windows/Linux tour (scripts/tauri-tour.sh): every view and tab of the app over the demo's whole timeline, one
// screenshot each into FLEET_SHOTS, so the two OSes' pairs can be laid side by side. Not a pass/fail test: it fails
// only when a view never shows.
const shots = process.env.FLEET_SHOTS ?? "./shots";
let n = 0;

async function shot(name: string) {
  // Animations and late fetches settle.
  await browser.pause(1200);
  n += 1;
  await browser.saveScreenshot(`${shots}/${String(n).padStart(2, "0")}-${name}.png`);
}

async function go(hash: string) {
  await browser.execute((h) => {
    window.location.hash = h;
  }, hash);
}

const FLEET = ["overview", "products", "personas", "agents", "limits", "pull-requests", "workflow", "instructions", "specialists", "skills", "pr-styles"];
const SETTINGS = ["app", "daemon", "tools", "log"];

describe("Tour", () => {
  it("visits every view", async () => {
    await expect($("h1=Filters")).toBeDisplayed({ wait: 30_000 });
    await browser.setWindowSize(1600, 900).catch(() => undefined);
    // The same theme on both OSes (System would follow each desktop's own): FLEET_THEME, dark by default.
    const theme = process.env.FLEET_THEME ?? "dark";
    await $(`aria/${theme[0]!.toUpperCase()}${theme.slice(1)} theme`).click();
    await shot("kanban");

    // A task's panel and each of its tabs.
    const card = $("article.card");
    if (await card.isExisting()) {
      await card.click();
      await shot("panel");
      const tabs = await $$('[role="tab"]');
      for (const tab of tabs) {
        const label = ((await tab.getText()) || "tab").toLowerCase().replace(/[^a-z0-9]+/g, "-");
        await tab.click();
        await shot(`panel-${label}`);
      }
      await browser.keys("Escape");
    }

    for (const [hash, name] of [["#tasks", "tasks"], ["#supervisor", "supervisor"], ["#releases", "releases"]]) {
      await go(hash);
      await shot(name);
    }
    for (const tab of FLEET) {
      await go(tab === "overview" ? "#fleet" : `#fleet?tab=${tab}`);
      await shot(`fleet-${tab}`);
    }
    for (const tab of SETTINGS) {
      await go(tab === "app" ? "#settings" : `#settings?tab=${tab}`);
      await shot(`settings-${tab}`);
    }

    // The new-task dialog, from the board.
    await go("#kanban");
    const add = $("button*=New task");
    if (await add.isExisting()) {
      await add.click();
      await shot("new-task");
      await browser.keys("Escape");
    }
  });
});
