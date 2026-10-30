import { act, render, screen, within } from "@testing-library/react";
import { vi } from "vitest";
import { App } from "../App";
import type { Task } from "../api/generated/Task";
import type { TaskList } from "../api/generated/TaskList";
import { FakeEventSource, NOW, agent, factory, fakeClient, persona, product, repoInfo, tasksList } from "../test/fixtures";
import { tasksHref } from "../shell/routes";

// The Tasks view's test harness, after egui's tests/tasks.rs helpers.

export function start(list: TaskList | (() => Promise<TaskList>) = tasksList(), over: Parameters<typeof fakeClient>[0] = {}) {
  const tasks = typeof list === "function" ? vi.fn(list) : vi.fn().mockResolvedValue(list);
  const client = fakeClient({ personas: vi.fn().mockResolvedValue({ personas: [persona("p")] }), tasks, ...over });
  render(<App client={client} eventSource={factory} nowSeconds={() => NOW} />);
  return { client, tasks };
}

export function go(href: string) {
  act(() => {
    window.location.hash = href;
    window.dispatchEvent(new HashChangeEvent("hashchange"));
  });
}

/** The Tasks view with `selected` picked, as egui's `tasks(sel)`. */
export async function tasks(selected: string, list?: TaskList, over?: Parameters<typeof fakeClient>[0]) {
  window.location.hash = tasksHref(undefined, selected);
  const started = start(list, over);
  await screen.findByRole("list", { name: "Tasks" });
  return started;
}

export const rows = () =>
  within(screen.getByRole("list", { name: "Tasks" }))
    .getAllByRole("button")
    .map((b) => b.getAttribute("aria-label"));
export const row = (name: string) => within(screen.getByRole("list", { name: "Tasks" })).getByRole("button", { name });
export const totals = () => screen.getByRole("region", { name: "Totals" });

export function snapshot(...agents: ReturnType<typeof agent>[]) {
  act(() => FakeEventSource.for("/api/events").emit("snapshot", { seq: 1, agents }, "1"));
}

export const products = (...list: ReturnType<typeof product>[]) => ({ products: vi.fn().mockResolvedValue({ products: list }) });
export const productP = product("p", "p", {
  repos: [repoInfo("core", { product: "p", url: null })],
  releases: [
    { id: 1, product: "p", version: "1.0", start: "2026-10-01", target: "2026-11-01", released_at: null },
    { id: 2, product: "p", version: "2.0", start: "2026-10-01", target: "2026-11-01", released_at: null },
  ],
});

/** egui's `app_with`: products p and q, `active` picked, these tasks and the usual orphan. */
export function withProducts(list: Task[], active: string) {
  localStorage.setItem("fleet.product", active);
  return [{ tasks: list, orphans: tasksList().orphans, clones_bytes: null }, products(productP, product("q"))] as const;
}

/** A promise the test settles by hand, as egui's tests hand results to the app. */
export function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** The footer's history, newest first. */
export async function history(): Promise<string[]> {
  const footer = screen.getByRole("contentinfo");
  await act(async () => within(footer).getByRole("button", { name: "History" }).click());
  return within(footer)
    .getAllByRole("listitem")
    .map((li) => li.textContent ?? "");
}
