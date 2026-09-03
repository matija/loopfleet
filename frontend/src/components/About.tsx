// The About panel, opened from the app menu's "About Loopfleet". macOS's own
// About panel can render a name, a version and a copyright line, but not a
// link — and the two things worth showing here (who makes Loopfleet, and where
// the source lives) are both links. So this is the app's own modal, drawn in
// the same tokens as the ⌘K palette, and the menu item opens it instead of the
// predefined one.

import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openExternal } from "../commands";

const VENDOR = "Tandoku d.o.o.";
const VENDOR_URL = "https://tandoku.hr";
const REPO_URL = "https://github.com/matija/loopfleet";

/// A link that leaves the app. Anchors are inert in the webview (a plain
/// `target="_blank"` navigates the app's own window), so every link here goes
/// through the `open_external` command instead.
function ExternalLink({ href, children }: { href: string; children: string }) {
  return (
    <button
      className="about__link"
      type="button"
      onClick={() => void openExternal(href)}
      title={href}
    >
      {children}
    </button>
  );
}

export function About({ onClose }: { onClose: () => void }) {
  // Read from the bundle rather than package.json: the version the user is
  // running is the one Tauri was built with, which is what a bug report needs.
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getVersion()
      .then((v) => {
        if (!cancelled) setVersion(v);
      })
      .catch(() => {
        // A version we can't read is not worth an error surface — the panel
        // simply omits the line.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      className="about__overlay"
      role="dialog"
      aria-modal="true"
      aria-label="About Loopfleet"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="about">
        <div className="about__body">
          <h1 className="about__name">Loopfleet</h1>
          {version && <p className="about__version">Version {version}</p>}
          <p className="about__vendor">
            by <ExternalLink href={VENDOR_URL}>{VENDOR}</ExternalLink>
          </p>
          <p className="about__repo">
            <ExternalLink href={REPO_URL}>Source on GitHub</ExternalLink>
          </p>
          <button className="about__close" type="button" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
