import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { FleetClient } from "./api/client";
import { detectNative, installAppNative } from "./native/native";
import { setChoiceStore } from "./shell/choices";
import { Startup } from "./startup/Startup";
import "./styles.css";

declare global {
  interface Window {
    /** Fleet endpoint override; empty means the daemon that served this page. */
    __FLEET_URL__?: string;
  }
}

const connect = (url: string) => new FleetClient(url);

async function start() {
  const native = await detectNative();
  installAppNative(native);
  // The board's choices come from gui.toml in the app; unreadable, they start empty, as in egui.
  if (native) setChoiceStore({ native, saved: await native.readChoices().catch(() => ({ persona: null, product: null, display: "compact" })) });
  // egui's animations setting is not in the web yet; the system's reduced-motion choice stands in for it.
  const animate = !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <Startup
        connect={connect}
        browserUrl={window.__FLEET_URL__ ?? import.meta.env.VITE_FLEET_URL ?? ""}
        native={native}
        animate={animate}
      >
        {(started) => <App client={started.client} native={native} wizard={started} />}
      </Startup>
    </StrictMode>,
  );
}

void start();
