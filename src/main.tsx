import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme } from "./lib/theme";
import { initAuth } from "./lib/auth";

applyTheme();

// The stored bearer token has to be in memory before the first session check,
// otherwise the app renders signed-out and never re-checks.
await initAuth();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
