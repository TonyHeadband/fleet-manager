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
