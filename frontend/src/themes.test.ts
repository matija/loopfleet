import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  applyTheme,
  DEFAULT_THEME_ID,
  isThemeId,
  previewThemeId,
  readStoredThemeId,
  resolveThemeId,
  storeThemeId,
  themeById,
  THEMES,
  THEME_STORAGE_KEY,
} from "./themes";

// No jsdom here either, so localStorage needs the same in-memory stand-in the
// other storage tests use. `throwOnAccess` simulates the private-mode /
// storage-disabled case, where touching localStorage throws.
class MemoryStorage {
  private store = new Map<string, string>();
  throwOnAccess = false;
  getItem(key: string): string | null {
    if (this.throwOnAccess) throw new Error("storage disabled");
    return this.store.has(key) ? this.store.get(key)! : null;
  }
  setItem(key: string, value: string): void {
    if (this.throwOnAccess) throw new Error("storage disabled");
    this.store.set(key, value);
  }
  clear(): void {
    this.store.clear();
  }
}

const storage = new MemoryStorage();
(globalThis as unknown as { localStorage: MemoryStorage }).localStorage =
  storage;

beforeEach(() => {
  storage.clear();
  storage.throwOnAccess = false;
});

// The vitest environment here is plain node (no jsdom), so there's no document
// to theme. applyTheme only needs setAttribute, so a minimal stand-in element
// is enough — and one is installed as globalThis.document.documentElement to
// exercise the default-argument path too.
class FakeElement {
  attrs = new Map<string, string>();
  setAttribute(name: string, value: string): void {
    this.attrs.set(name, value);
  }
}

function withDocument(root: FakeElement, run: () => void): void {
  const g = globalThis as { document?: unknown };
  const had = "document" in g;
  const previous = g.document;
  g.document = { documentElement: root };
  try {
    run();
  } finally {
    if (had) g.document = previous;
    else delete g.document;
  }
}

afterEach(() => {
  delete (globalThis as { document?: unknown }).document;
});

describe("THEMES", () => {
  it("lists every theme as an { id, label } pair", () => {
    expect(THEMES).toEqual([
      { id: "dark", label: "Dark" },
      { id: "rose-pine-moon", label: "Rosé Pine Moon" },
      { id: "github-dark", label: "GitHub Dark" },
      { id: "github-light", label: "GitHub Light" },
      { id: "tokyo-night", label: "Tokyo Night" },
      { id: "tokyo-night-storm", label: "Tokyo Night Storm" },
      { id: "tokyo-night-light", label: "Tokyo Night Light" },
    ]);
  });

  it("has unique ids and non-empty labels", () => {
    const ids = THEMES.map((theme) => theme.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const theme of THEMES) {
      expect(theme.label.length).toBeGreaterThan(0);
    }
  });

  it("includes the default theme", () => {
    expect(THEMES.map((theme) => theme.id)).toContain(DEFAULT_THEME_ID);
  });
});

describe("isThemeId", () => {
  it("accepts registered ids", () => {
    expect(isThemeId("dark")).toBe(true);
    expect(isThemeId("rose-pine-moon")).toBe(true);
    expect(isThemeId("github-dark")).toBe(true);
    expect(isThemeId("github-light")).toBe(true);
    expect(isThemeId("tokyo-night")).toBe(true);
    expect(isThemeId("tokyo-night-storm")).toBe(true);
    expect(isThemeId("tokyo-night-light")).toBe(true);
  });

  it("rejects anything else", () => {
    for (const value of ["", "light", "DARK", null, undefined, 3, {}, ["dark"]]) {
      expect(isThemeId(value)).toBe(false);
    }
  });
});

describe("resolveThemeId", () => {
  it("passes a known id through", () => {
    expect(resolveThemeId("rose-pine-moon")).toBe("rose-pine-moon");
  });

  it("falls back to the default for a missing preference", () => {
    expect(resolveThemeId(null)).toBe(DEFAULT_THEME_ID);
    expect(resolveThemeId(undefined)).toBe(DEFAULT_THEME_ID);
  });

  it("falls back to the default for an id no longer in the registry", () => {
    expect(resolveThemeId("solarized")).toBe(DEFAULT_THEME_ID);
  });

  it("falls back to the default for non-string values", () => {
    expect(resolveThemeId({ id: "dark" })).toBe(DEFAULT_THEME_ID);
    expect(resolveThemeId(7)).toBe(DEFAULT_THEME_ID);
  });
});

describe("themeById", () => {
  it("returns the registry entry", () => {
    expect(themeById("rose-pine-moon").label).toBe("Rosé Pine Moon");
  });
});

describe("previewThemeId", () => {
  it("shows the highlighted theme while one is highlighted", () => {
    expect(previewThemeId("rose-pine-moon", "dark")).toBe("rose-pine-moon");
  });

  it("falls back to the applied theme when nothing is highlighted", () => {
    expect(previewThemeId(null, "rose-pine-moon")).toBe("rose-pine-moon");
    expect(previewThemeId(undefined, "dark")).toBe("dark");
  });

  it("falls back to the applied theme for an unknown highlight", () => {
    expect(previewThemeId("solarized", "rose-pine-moon")).toBe(
      "rose-pine-moon",
    );
    expect(previewThemeId(7, "dark")).toBe("dark");
  });

  it("does not fall back to the default when the applied theme differs", () => {
    // The preview shows what the app is wearing, not what it shipped with.
    expect(previewThemeId(null, "rose-pine-moon")).not.toBe(DEFAULT_THEME_ID);
  });
});

describe("applyTheme", () => {
  it("sets data-theme on the given root and returns the applied id", () => {
    const root = new FakeElement();
    expect(applyTheme("rose-pine-moon", root as unknown as Element)).toBe(
      "rose-pine-moon",
    );
    expect(root.attrs.get("data-theme")).toBe("rose-pine-moon");
  });

  it("writes the default id when the stored value is invalid", () => {
    const root = new FakeElement();
    expect(applyTheme("solarized", root as unknown as Element)).toBe(
      DEFAULT_THEME_ID,
    );
    expect(root.attrs.get("data-theme")).toBe(DEFAULT_THEME_ID);
  });

  it("overwrites a previously applied theme", () => {
    const root = new FakeElement();
    applyTheme("rose-pine-moon", root as unknown as Element);
    applyTheme("dark", root as unknown as Element);
    expect(root.attrs.get("data-theme")).toBe("dark");
  });

  it("defaults to the document root", () => {
    const root = new FakeElement();
    withDocument(root, () => {
      expect(applyTheme("rose-pine-moon")).toBe("rose-pine-moon");
    });
    expect(root.attrs.get("data-theme")).toBe("rose-pine-moon");
  });

  it("is a no-op outside a DOM instead of throwing", () => {
    expect(() => applyTheme("dark")).not.toThrow();
    expect(applyTheme("dark")).toBe("dark");
  });
});

describe("readStoredThemeId", () => {
  it("returns the stored id", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "rose-pine-moon");
    expect(readStoredThemeId()).toBe("rose-pine-moon");
  });

  it("falls back to the default when nothing is stored", () => {
    expect(readStoredThemeId()).toBe(DEFAULT_THEME_ID);
  });

  it("falls back to the default for a theme that no longer exists", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "solarized");
    expect(readStoredThemeId()).toBe(DEFAULT_THEME_ID);
  });

  it("falls back to the default when localStorage throws", () => {
    storage.throwOnAccess = true;
    expect(readStoredThemeId()).toBe(DEFAULT_THEME_ID);
  });
});

describe("storeThemeId", () => {
  it("round-trips through readStoredThemeId", () => {
    storeThemeId("rose-pine-moon");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("rose-pine-moon");
    expect(readStoredThemeId()).toBe("rose-pine-moon");
  });

  it("swallows a storage failure", () => {
    storage.throwOnAccess = true;
    expect(() => storeThemeId("dark")).not.toThrow();
  });
});
