import React from "react";
import ReactDOM from "react-dom/client";
import { Provider } from "react-redux";
import { MantineProvider } from "@mantine/core";
import "@mantine/core/styles.css";
import "./styles.css";
import { theme } from "./theme";
import { store } from "./store";
import { Surface } from "./surfaces/Surface";
import { ErrorBoundary } from "./ui/ErrorBoundary";
import { ThemeSync, useThemeRerender } from "./ui/ThemeSync";
import { applyScheme } from "./ui/tokens";
import { cachedTheme } from "./settings";

// Seed the JS palette from the cached theme before first paint (flash-free); the
// backend-truth scheme is re-applied by ThemeSync once settings load.
const startTheme = cachedTheme();
applyScheme(startTheme);

// The app tree lives in a component that subscribes to theme swaps: a swap re-runs
// this render, so `<Surface/>` and everything under it are fresh elements and
// re-render with the new palette (state preserved - no remount).
function App() {
  useThemeRerender();
  return (
    <Provider store={store}>
      <ErrorBoundary>
        <Surface />
      </ErrorBoundary>
    </Provider>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <MantineProvider theme={theme} defaultColorScheme={startTheme}>
      <ThemeSync />
      <App />
    </MantineProvider>
  </React.StrictMode>
);
