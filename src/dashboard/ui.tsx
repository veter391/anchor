//! Shared dashboard UI primitives — tokens, panel/card styles, the nav icon
//! set. Keeps the five views consistent and DRY. Palette is the owner-locked
//! one from styles.css (06_DESIGN); this only composes it.

import type { CSSProperties, ReactNode } from "react";

export const panel: CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border)",
  borderRadius: 12,
  padding: 18,
};

export const btn: CSSProperties = {
  background: "var(--bg)",
  color: "var(--accent)",
  border: "1px solid var(--accent)",
  borderRadius: 8,
  padding: "8px 16px",
  fontSize: 14,
  cursor: "pointer",
  transition: "background 160ms, border-color 160ms, color 160ms",
};

export const btnGhost: CSSProperties = {
  ...btn,
  border: "1px solid var(--border)",
  color: "var(--text-muted)",
};

/** A soft branded backdrop — faint accent glow top-left, never busy (owner:
 *  no bare backgrounds, but nothing that pulls the eye). */
export const pageBackdrop: CSSProperties = {
  background:
    "radial-gradient(1200px 500px at 8% -10%, rgba(79,209,197,0.06), transparent 60%)," +
    "radial-gradient(900px 500px at 100% 0%, rgba(180,142,222,0.05), transparent 55%)," +
    "var(--bg-elevated)",
};

export function SectionTitle({ children, hint }: { children: ReactNode; hint?: string }) {
  return (
    <div style={{ marginBottom: 12 }}>
      <h3
        style={{
          margin: 0,
          fontSize: 12,
          letterSpacing: "0.08em",
          textTransform: "uppercase",
          color: "var(--text-muted)",
        }}
      >
        {children}
      </h3>
      {hint && (
        <p style={{ margin: "4px 0 0", fontSize: 13, color: "var(--text-muted)" }}>{hint}</p>
      )}
    </div>
  );
}

export type NavKey = "general" | "sessions" | "cards" | "settings" | "about";

/** Minimal stroke icons for the left rail — one <path> each, currentColor. */
export function NavIcon({ name, size = 22 }: { name: NavKey; size?: number }) {
  const common = {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.7,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };
  switch (name) {
    case "general":
      return (
        <svg {...common}>
          <path d="M3 10.5 12 3l9 7.5" />
          <path d="M5 9.5V20h14V9.5" />
          <path d="M9.5 20v-6h5v6" />
        </svg>
      );
    case "sessions":
      return (
        <svg {...common}>
          <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
        </svg>
      );
    case "cards":
      return (
        <svg {...common}>
          <rect x="3" y="5" width="14" height="14" rx="2" />
          <path d="M7 9h6M7 12h6M7 15h4" />
          <path d="M17 8h3a1 1 0 0 1 1 1v8" />
        </svg>
      );
    case "settings":
      return (
        <svg {...common}>
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
      );
    case "about":
      return (
        <svg {...common}>
          <circle cx="12" cy="12" r="9" />
          <path d="M12 16v-5M12 8h.01" />
        </svg>
      );
  }
}

/** The little fish-anchor wordmark lockup used on General + rail top. */
export function Wordmark({ size = 20 }: { size?: number }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
      <span
        aria-hidden
        style={{
          width: size,
          height: size,
          borderRadius: 6,
          background: "var(--accent)",
          display: "inline-block",
          maskImage:
            "radial-gradient(circle at 50% 62%, transparent 20%, black 21%)",
          WebkitMaskImage:
            "radial-gradient(circle at 50% 62%, transparent 20%, black 21%)",
        }}
      />
      <span style={{ fontWeight: 600, letterSpacing: "0.01em" }}>Anchor</span>
    </span>
  );
}
