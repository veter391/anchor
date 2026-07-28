//! Dashboard shell — the left icon rail + the active view. Owner vision
//! (09_PLAN Phase 6): a small friendly desktop app; a student opens it and
//! immediately understands what it is and where to click. Warm, light,
//! never a wall of monospace. Debug hides behind a Settings toggle.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Mark, NavIcon, pageBackdrop, applyAppearance, type NavKey } from "./ui";
import { General } from "./views/General";
import { Sessions } from "./views/Sessions";
import { Cards } from "./views/Cards";
import { Settings } from "./views/Settings";
import { About } from "./views/About";
import { ConsentModal } from "./ConsentModal";
import { Setup } from "./Setup";

const NAV: { key: NavKey; label: string }[] = [
  { key: "general", label: "General" },
  { key: "sessions", label: "Sessions" },
  { key: "cards", label: "Cards" },
  { key: "settings", label: "Settings" },
  { key: "about", label: "About" },
];

export function App() {
  const [view, setView] = useState<NavKey>("general");
  const [version, setVersion] = useState("");
  // null = still loading; false = show the first-run consent screen.
  const [consent, setConsent] = useState<boolean | null>(null);
  // true = no speech model yet → download the default automatically after consent.
  const [needsSetup, setNeedsSetup] = useState(false);

  useEffect(() => {
    invoke<{ accent: string; theme: string }>("get_appearance")
      .then(applyAppearance)
      .catch(() => {});
    invoke<string>("app_version").then(setVersion).catch(() => {});
    invoke<boolean>("get_consent")
      .then(setConsent)
      .catch(() => setConsent(true)); // fail-open: never lock the user out
    invoke<{ installed: boolean }[]>("list_asr_models")
      .then((rows) => setNeedsSetup(rows.length > 0 && rows.every((r) => !r.installed)))
      .catch(() => {});
  }, []);

  return (
    <div className="grain" style={{ display: "flex", minHeight: "100vh", color: "var(--text)" }}>
      {consent === false && (
        <ConsentModal
          onAccept={() => {
            invoke("accept_consent").catch(() => {});
            setConsent(true);
          }}
        />
      )}
      {consent === true && needsSetup && (
        <Setup onReady={() => setNeedsSetup(false)} onSkip={() => setNeedsSetup(false)} />
      )}
      {/* Left icon rail */}
      <nav
        style={{
          width: 84,
          flexShrink: 0,
          background: "var(--bg)",
          borderRight: "1px solid var(--border-soft)",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          paddingTop: 16,
          gap: 4,
          position: "sticky",
          top: 0,
          height: "100vh",
          zIndex: 1,
        }}
      >
        <div style={{ marginBottom: 14 }}>
          <Mark size={32} />
        </div>
        {NAV.map((n) => {
          const active = view === n.key;
          return (
            <button
              key={n.key}
              onClick={() => setView(n.key)}
              title={n.label}
              style={{
                width: 68,
                padding: "10px 0 8px",
                borderRadius: 12,
                border: "none",
                cursor: "pointer",
                display: "flex",
                flexDirection: "column",
                alignItems: "center",
                gap: 5,
                background: active ? "var(--accent-bg)" : "transparent",
                color: active ? "var(--accent)" : "var(--text-muted)",
                transition: "background 160ms, color 160ms",
              }}
            >
              <NavIcon name={n.key} />
              <span style={{ fontSize: 11, letterSpacing: "0.01em" }}>{n.label}</span>
            </button>
          );
        })}
      </nav>

      {/* Content */}
      <main
        style={{
          flex: 1,
          ...pageBackdrop,
          padding: "34px 40px 60px",
          overflowY: "auto",
          height: "100vh",
          position: "relative",
          zIndex: 1,
        }}
      >
        <div className="rise-in" key={view}>
          {view === "general" && <General onNavigate={setView} />}
          {view === "sessions" && <Sessions />}
          {view === "cards" && <Cards />}
          {view === "settings" && <Settings />}
          {view === "about" && <About />}
        </div>

        {/* Version badge, bottom-right (Handy-style). Update pill wires later. */}
        {version && (
          <div
            style={{
              position: "fixed",
              right: 16,
              bottom: 12,
              fontSize: 11,
              color: "var(--text-dim)",
              userSelect: "none",
            }}
            title="Anchor version"
          >
            v{version}
          </div>
        )}
      </main>
    </div>
  );
}
