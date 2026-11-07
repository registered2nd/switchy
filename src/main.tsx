import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-sans-condensed/500.css";
import "@fontsource/ibm-plex-sans-condensed/600.css";
import "./index.css";
// i18n setup
import i18n from "./i18n";
import { QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "@/components/theme-provider";
import { queryClient } from "@/lib/query";
import { Toaster } from "@/components/ui/sonner";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import { exit } from "@tauri-apps/plugin-process";

// Add a platform body class for platform-specific styles
try {
  const ua = navigator.userAgent || "";
  const plat = (navigator.platform || "").toLowerCase();
  const isMac = /mac/i.test(ua) || plat.includes("mac");
  if (isMac) {
    document.body.classList.add("is-mac");
  }
} catch {
  // Ignore platform detection failures
}

// Config load error payload
interface ConfigLoadErrorPayload {
  path?: string;
  error?: string;
}

/**
 * Handle a config load failure: show the error and force the app to quit
 * No "cancel" option, since the app cannot run with a corrupted config
 */
async function handleConfigLoadError(
  payload: ConfigLoadErrorPayload | null,
): Promise<void> {
  const path = payload?.path ?? "~/.switchy/config.json";
  const detail = payload?.error ?? "Unknown error";

  await message(
    i18n.t("errors.configLoadFailedMessage", {
      path,
      detail,
      defaultValue:
        "Unable to read configuration file:\n{{path}}\n\nError details:\n{{detail}}\n\nPlease check if the JSON is valid, or restore from a backup file (e.g., config.json.bak) in the same directory.\n\nThe app will exit so you can fix this.",
    }),
    {
      title: i18n.t("errors.configLoadFailedTitle", {
        defaultValue: "Configuration Load Failed",
      }),
      kind: "error",
    },
  );

  await exit(1);
}

// Listen for the backend config load error event: warn the user and force quit, without touching any config file
try {
  void listen("configLoadError", async (evt) => {
    await handleConfigLoadError(evt.payload as ConfigLoadErrorPayload | null);
  });
} catch (e) {
  // Ignore event subscription failures (e.g. outside Tauri)
  console.error("Failed to subscribe to the configLoadError event", e);
}

async function bootstrap() {
  // Ask the backend for init errors early in startup to avoid an event race
  try {
    const initError = (await invoke(
      "get_init_error",
    )) as ConfigLoadErrorPayload | null;
    if (initError && (initError.path || initError.error)) {
      await handleConfigLoadError(initError);
      // Note: never reached, since exit(1) ends the process
      return;
    }
  } catch (e) {
    // Ignore fetch errors and keep rendering
    console.error("Failed to fetch init error", e);
  }

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <QueryClientProvider client={queryClient}>
        <ThemeProvider defaultTheme="system" storageKey="switchy-theme">
          <App />
          <Toaster />
        </ThemeProvider>
      </QueryClientProvider>
    </React.StrictMode>,
  );
}

void bootstrap();
