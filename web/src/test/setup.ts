import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { resetPreferences } from "../shell/preferences";

// The preferences are one copy per window; each test is a window of its own.
afterEach(() => resetPreferences());
