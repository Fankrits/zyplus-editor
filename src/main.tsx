import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme } from "./lib/theme";
import { initAuth } from "./lib/auth";
import { initWebFs } from "./lib/fs";

applyTheme();

// Both have to finish before the first render — the token so the session check
// sees it rather than rendering signed-out forever, the notes because the first
// render reads them — but neither waits on the other, and run in series they made
// the window wait for a keychain read and an IndexedDB open one after the other.
await Promise.all([initAuth(), initWebFs()]);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
