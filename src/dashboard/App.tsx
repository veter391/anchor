//! Dashboard shell — the left icon rail + the active view. Owner vision
//! (09_PLAN Phase 6): a small friendly desktop app; a student opens it and
//! immediately understands what it is and where to click. Light everywhere
//! except Settings; the debug surfaces hide behind a Settings toggle.

import { useState } from "react";
import { NavIcon, pageBackdrop, type NavKey } from "./ui";
import { General } from "./views/General";
import { Sessions } from "./views/Sessions";
import { Cards } from "./views/Cards";
import { Settings } from "./views/Settings";
import { About } from "./views/About";

const NAV: { key: NavKey; label: string }[] = [
  { key: "general", label: "General" },
  { key: "sessions", label: "Sessions" },
  { key: "cards", label: "Cards" },
  { key: "settings", label: "Settings" },
  { key: "about", label: "About" },
];

export function App() {
  const [view, setView] = useState<NavKey>("general");

  return (
    <div style={{ display: "flex", minHeight: "100vh", color: "var(--text)" }}>
      {/* Left icon rail */}
      <nav
        style={{
          width: 84,
          flexShrink: 0,
          background: "var(--bg)",
          borderRight: "1px solid var(--border)",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          paddingTop: 16,
          gap: 6,
          position: "sticky",
          top: 0,
          height: "100vh",
        }}
      >
        <div
          aria-hidden
          style={{
            width: 30,
            height: 30,
            borderRadius: 8,
            background: "var(--accent)",
            marginBottom: 14,
            maskImage: "radial-gradient(circle at 50% 62%, transparent 22%, black 23%)",
            WebkitMaskImage: "radial-gradient(circle at 50% 62%, transparent 22%, black 23%)",
          }}
        />
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
                borderRadius: 10,
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
      <main style={{ flex: 1, ...pageBackdrop, padding: "32px 36px", overflowY: "auto", height: "100vh" }}>
        {view === "general" && <General onNavigate={setView} />}
        {view === "sessions" && <Sessions />}
        {view === "cards" && <Cards />}
        {view === "settings" && <Settings />}
        {view === "about" && <About />}
      </main>
    </div>
  );
}
