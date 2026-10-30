import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { FleetClient } from "./api/client";
import { DEFAULT_CHOICES, detectNative, installAppNative } from "./native/native";
import { setChoiceStore } from "./shell/choices";
import { applyPreferences, readPreferences } from "./shell/preferences";
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
  if (native) setChoiceStore({ native, saved: await native.readChoices().catch(() => DEFAULT_CHOICES) });
  // Before the first paint, so nothing shows in the default look first. egui applies fonts and animations only once
  // the board starts; here the intro and the first run get them too.
  const preferences = readPreferences();
  applyPreferences(preferences);
  const animate = preferences.animations;
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
