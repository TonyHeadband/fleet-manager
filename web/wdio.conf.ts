// The Tauri app's smoke test (e2e/), run by scripts/tauri-smoke.sh in CI: tauri-driver starts the app through
// WebKitWebDriver on Linux and msedgedriver on Windows (FLEET_NATIVE_DRIVER), which this config talks to. The script
// sets FLEET_TAURI_APP and the throwaway home.
import { spawn, type ChildProcess } from "node:child_process";

let driver: ChildProcess | undefined;

export const config: WebdriverIO.Config = {
  runner: "local",
  hostname: "127.0.0.1",
  port: 4444,
  // CI runs the smoke specs; FLEET_E2E_SPECS names others (e2e/manual/, see scripts/tauri-check.sh).
  specs: [process.env.FLEET_E2E_SPECS ?? "./e2e/**/*.e2e.ts"],
  maxInstances: 1,
  capabilities: [{ maxInstances: 1, "tauri:options": { application: process.env.FLEET_TAURI_APP } } as WebdriverIO.Capabilities],
  logLevel: "warn",
  reporters: ["spec"],
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 60_000 },
  beforeSession: () => {
    // Windows names msedgedriver (WebView2's), matched to the installed WebView2; Linux finds WebKitWebDriver itself.
    const native = process.env.FLEET_NATIVE_DRIVER;
    driver = spawn("tauri-driver", native ? ["--native-driver", native] : [], {
      stdio: [null, process.stdout, process.stderr],
    });
  },
  afterSession: () => {
    driver?.kill();
  },
};
