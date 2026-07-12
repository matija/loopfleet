import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource/readex-pro/400.css";
import "@fontsource/readex-pro/500.css";
import "@fontsource/readex-pro/600.css";
import "@fontsource/readex-pro/700.css";
import "./tokens.css";
import "./shell.css";
import "./tabs.css";
import "./commandbar.css";
import "./panels.css";
import "./plan.css";
import "./dock.css";
import "./runview.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
