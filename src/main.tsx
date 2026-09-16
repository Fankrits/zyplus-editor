import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme } from "./lib/theme";
import { initAuth } from "./lib/auth";
import { initWebFs } from "./lib/fs";

applyTheme();

// The stored bearer token has to be in memory before the first session check,
// otherwise the app renders signed-out and never re-checks.
await initAuth();
// The web build's notes load from browser storage, and the first render reads them.
await initWebFs();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
