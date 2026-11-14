import { afterEach, beforeEach, type MockInstance, vi } from "vitest";

const WINDOWS_USER_AGENT = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";

/** Makes `platform()` report Windows for the tests of the enclosing `describe`. */
export function useWindowsUserAgent() {
  let spy: MockInstance | undefined;
  beforeEach(() => {
    spy = vi.spyOn(navigator, "userAgent", "get").mockReturnValue(WINDOWS_USER_AGENT);
  });
  afterEach(() => spy?.mockRestore());
}
