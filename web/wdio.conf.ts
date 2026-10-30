// The Tauri app's smoke test (e2e/), run by scripts/tauri-smoke.sh in CI: tauri-driver starts the app through
// WebKitWebDriver, which this config talks to. The script sets FLEET_TAURI_APP and the throwaway HOME.
import { spawn, type ChildProcess } from "node:child_process";

let driver: ChildProcess | undefined;

export const config: WebdriverIO.Config = {
  runner: "local",
  hostname: "127.0.0.1",
  port: 4444,
  specs: ["./e2e/**/*.e2e.ts"],
  maxInstances: 1,
  capabilities: [{ maxInstances: 1, "tauri:options": { application: process.env.FLEET_TAURI_APP } } as WebdriverIO.Capabilities],
  logLevel: "warn",
  reporters: ["spec"],
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 60_000 },
  beforeSession: () => {
    driver = spawn("tauri-driver", [], { stdio: [null, process.stdout, process.stderr] });
  },
  afterSession: () => {
    driver?.kill();
  },
};
