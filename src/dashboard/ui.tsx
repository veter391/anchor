//! Shared dashboard UI primitives — tokens, panel/card styles, the nav icon
//! set, the brand mark. Palette is token-driven (styles.css); accent + theme
//! switch live via data-accent / data-theme on <html>.

import type { CSSProperties, ReactNode } from "react";
import markUrl from "../assets/anchor-mark.png";

export const panel: CSSProperties = {
  background: "var(--bg-soft)",
  border: "1px solid var(--border-soft)",
  borderRadius: "var(--radius)",
  padding: 18,
  boxShadow: "var(--shadow-soft)",
};

export const btn: CSSProperties = {
  background: "var(--accent)",
  color: "#12100f",
  border: "1px solid transparent",
  borderRadius: 10,
  padding: "9px 16px",
  fontSize: 14,
  fontWeight: 600,
  cursor: "pointer",
  transition: "filter 160ms, transform 160ms",
};

export const btnGhost: CSSProperties = {
  background: "transparent",
  color: "var(--text)",
  border: "1px solid var(--border)",
  borderRadius: 10,
  padding: "9px 16px",
  fontSize: 14,
  cursor: "pointer",
  transition: "border-color 160ms, background 160ms, color 160ms",
};

/** A warm branded backdrop — a soft accent glow, never busy. */
export const pageBackdrop: CSSProperties = {
  background:
    "radial-gradient(1100px 460px at 6% -12%, var(--accent-glow), transparent 60%)," +
    "radial-gradient(760px 420px at 102% 4%, rgba(180,142,222,0.05), transparent 55%)," +
    "var(--bg-elevated)",
};

export function SectionTitle({
  children,
  hint,
  emoji,
}: {
  children: ReactNode;
  hint?: string;
  emoji?: string;
}) {
  return (
    <div style={{ marginBottom: 12 }}>
      <h3
        style={{
          margin: 0,
          fontSize: 12,
          letterSpacing: "0.09em",
          textTransform: "uppercase",
          color: "var(--text-muted)",
          display: "flex",
          alignItems: "center",
          gap: 7,
        }}
      >
        {emoji && (
          <span aria-hidden style={{ fontSize: 13 }}>
            {emoji}
          </span>
        )}
        {children}
      </h3>
      {hint && (
        <p style={{ margin: "5px 0 0", fontSize: 13, color: "var(--text-muted)" }}>{hint}</p>
      )}
    </div>
  );
}

/** The app mark (teal fish-anchor — brand-locked, does not follow the accent). */
export function Mark({ size = 28 }: { size?: number }) {
  return (
    <img
      src={markUrl}
      alt="Anchor"
      width={size}
      height={size}
      style={{ display: "block", borderRadius: size * 0.28 }}
    />
  );
}

export function Wordmark({ size = 24 }: { size?: number }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 9 }}>
      <Mark size={size} />
      <span style={{ fontWeight: 600, letterSpacing: "0.01em", fontSize: size * 0.72 }}>
        Anchor
      </span>
    </span>
  );
}

/** Applies accent + theme to <html> (both are token-switch attributes). Any
 *  component can call it — the document root is shared. */
export function applyAppearance(a: { accent?: string; theme?: string }) {
  const el = document.documentElement;
  if (a.accent) el.setAttribute("data-accent", a.accent);
  if (a.theme) el.setAttribute("data-theme", a.theme);
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
