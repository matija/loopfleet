import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./tokens.css";
import "./shell.css";
import "./panels.css";
import "./plan.css";
import "./dock.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
