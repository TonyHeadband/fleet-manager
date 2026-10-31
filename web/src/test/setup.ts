import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { setChoiceStore } from "../shell/choices";
import { resetPreferences } from "../shell/preferences";

// The choices and preferences are one copy per window; each test is a window of its own.
afterEach(() => {
  setChoiceStore(null);
  resetPreferences();
  sessionStorage.clear();
});

// jsdom has no AnimationEvent, so a fired animationend would lose its animationName; the canvas's entry reads it.
if (typeof window.AnimationEvent === "undefined") {
  class AnimationEventShim extends Event {
    animationName: string;
    constructor(type: string, init: AnimationEventInit = {}) {
      super(type, init);
      this.animationName = init.animationName ?? "";
    }
  }
  window.AnimationEvent = AnimationEventShim as unknown as typeof AnimationEvent;
}
