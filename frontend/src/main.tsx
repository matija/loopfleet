import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource-variable/inter";
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
import "./segmented-control.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
