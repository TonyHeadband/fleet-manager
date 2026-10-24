import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { FleetClient } from "./api/client";
import "./styles.css";

declare global {
  interface Window {
    /** Fleet endpoint override; empty means the daemon that served this page. */
    __FLEET_URL__?: string;
  }
}

const client = new FleetClient(window.__FLEET_URL__ ?? import.meta.env.VITE_FLEET_URL ?? "");
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App client={client} />
  </StrictMode>,
);
