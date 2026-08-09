import { beforeEach, describe, expect, it } from "vitest";
import { readLaunchPrefs, writeLaunchPrefs } from "./launchPrefs";

// The vitest environment here is plain node (no jsdom), so localStorage isn't
// globally available. A minimal in-memory stand-in is enough to exercise the
// module under test — it only needs getItem/setItem.
class MemoryStorage {
  private store = new Map<string, string>();
  getItem(key: string): string | null {
    return this.store.has(key) ? this.store.get(key)! : null;
  }
  setItem(key: string, value: string): void {
    this.store.set(key, value);
  }
  clear(): void {
    this.store.clear();
  }
}

(globalThis as unknown as { localStorage: MemoryStorage }).localStorage =
  new MemoryStorage();

const PROJECT_ID = "proj-1";
const INSTALLED = ["claude", "codex"];

beforeEach(() => {
  localStorage.clear();
});

describe("readLaunchPrefs", () => {
  it("defaults to claude/1 when nothing has been stored", () => {
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "claude",
      passes: 1,
    });
  });

  it("returns a previously written, still-installed preference", () => {
    writeLaunchPrefs(PROJECT_ID, { agent: "codex", passes: 3 });
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "codex",
      passes: 3,
    });
  });

  it("is scoped per project", () => {
    writeLaunchPrefs("proj-1", { agent: "codex", passes: 3 });
    expect(readLaunchPrefs("proj-2", INSTALLED)).toEqual({
      agent: "claude",
      passes: 1,
    });
  });

  it("falls back to the default agent when the stored agent is no longer installed", () => {
    writeLaunchPrefs(PROJECT_ID, { agent: "gemini", passes: 2 });
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "claude",
      passes: 2,
    });
  });

  it("tolerates malformed JSON", () => {
    localStorage.setItem(`loopfleet.launch.${PROJECT_ID}`, "{not valid json");
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "claude",
      passes: 1,
    });
  });

  it("tolerates a JSON value that isn't an object", () => {
    localStorage.setItem(`loopfleet.launch.${PROJECT_ID}`, "42");
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "claude",
      passes: 1,
    });
  });

  it("tolerates a null-valued agent/passes fields", () => {
    localStorage.setItem(
      `loopfleet.launch.${PROJECT_ID}`,
      JSON.stringify({ agent: null, passes: null }),
    );
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "claude",
      passes: 1,
    });
  });

  it("tolerates a non-numeric or sub-1 passes value", () => {
    localStorage.setItem(
      `loopfleet.launch.${PROJECT_ID}`,
      JSON.stringify({ agent: "codex", passes: "three" }),
    );
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "codex",
      passes: 1,
    });

    localStorage.setItem(
      `loopfleet.launch.${PROJECT_ID}`,
      JSON.stringify({ agent: "codex", passes: 0 }),
    );
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "codex",
      passes: 1,
    });
  });

  it("floors a fractional passes value", () => {
    localStorage.setItem(
      `loopfleet.launch.${PROJECT_ID}`,
      JSON.stringify({ agent: "codex", passes: 2.9 }),
    );
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "codex",
      passes: 2,
    });
  });

  it("ignores an empty installed list, always falling back to the default agent", () => {
    writeLaunchPrefs(PROJECT_ID, { agent: "claude", passes: 4 });
    expect(readLaunchPrefs(PROJECT_ID, [])).toEqual({
      agent: "claude",
      passes: 4,
    });
  });
});

describe("writeLaunchPrefs", () => {
  it("round-trips through readLaunchPrefs", () => {
    writeLaunchPrefs(PROJECT_ID, { agent: "codex", passes: 5 });
    expect(readLaunchPrefs(PROJECT_ID, INSTALLED)).toEqual({
      agent: "codex",
      passes: 5,
    });
  });
});
