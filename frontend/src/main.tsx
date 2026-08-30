import { emit } from "@tauri-apps/api/event";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource-variable/readex-pro";
import "./tokens.css";
import "./shell.css";
import "./panels.css";
import "./plan.css";
import "./prd.css";
import "./empty.css";
import "./dock.css";
import "./runview.css";
import "./status.css";
import "./grid.css";
import "./palette.css";
import "./toolbar.css";
import "./popover.css";
import "./hint.css";
import "./select.css";
import "./number-field.css";
// After status.css and dock.css: the theme preview reuses their run-status
// classes and tightens a couple of their layout rules for miniature scale.
import "./theme-preview.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// Signals that React's initial commit has landed. This deliberately does not
// wait for a paint: the window starts hidden (see tauri.conf.json), and a
// hidden WKWebView never paints, so a requestAnimationFrame callback here
// would never run and the window would stay invisible forever.
void emit("frontend-ready");
