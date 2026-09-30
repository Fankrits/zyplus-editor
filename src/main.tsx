import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme } from "./lib/theme";
import { initWebFs } from "./lib/fs";

applyTheme();

// The notes have to be loaded before the first render, because it reads them.
// initWebFs reports its own failures; whatever escapes it must not blank the app.
try {
  await initWebFs();
} catch (err) {
  console.error("Could not load the stored notes:", err);
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
