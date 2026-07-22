import { forwardRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type BulletState = "covered" | "next" | "uncovered";
export type CardSource = "prepared" | "assembled" | "context" | "panic";

export interface CardData {
  title: string;
  source: CardSource;
  bullets: { text: string; state: BulletState; provenance?: "model_knowledge" }[];
}

/** The overlay card. Glance-legibility rules from Documents/06_DESIGN.md:
 *  one card, max 6 bullets, 18px bullets, covered dims, next gets the accent
 *  bar, motion limited to colour shifts. */
export const Card = forwardRef<HTMLDivElement, { card: CardData }>(
  function Card({ card }, ref) {
    return (
      <div
        ref={ref}
        style={{
          background: "var(--bg)",
          border: "1px solid var(--border)",
          borderTop:
            card.source === "assembled"
              ? "2px solid var(--assembled)"
              : card.source === "panic"
                ? "2px solid var(--panic)"
                : "1px solid var(--border)",
          borderRadius: 10,
          padding: "10px 14px",
        }}
      >
        <div
          onMouseDown={(e) => {
            if (e.button === 0) void getCurrentWindow().startDragging();
          }}
          style={{
            fontSize: "var(--size-title)",
            color: "var(--text-muted)",
            textTransform: "uppercase",
            letterSpacing: "0.08em",
            cursor: "move",
            paddingBottom: 6,
            borderBottom: "1px solid var(--border)",
            marginBottom: 8,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {card.source === "assembled" && (
            <span style={{ color: "var(--assembled)" }}>ASSEMBLED LIVE · </span>
          )}
          {card.title}
        </div>

        {card.bullets.slice(0, 6).map((b, i) => (
          <div
            key={i}
            style={{
              fontSize: "var(--size-bullet)",
              lineHeight: "var(--leading)",
              color: b.state === "covered" ? "var(--text-dim)" : "var(--text)",
              background: b.state === "next" ? "var(--accent-bg)" : "transparent",
              borderLeft:
                b.state === "next"
                  ? "3px solid var(--accent)"
                  : "3px solid transparent",
              paddingLeft: 8,
              transition: "color 120ms, background 120ms",
            }}
          >
            {b.state === "covered" && <span style={{ marginRight: 6 }}>✓</span>}
            {b.provenance === "model_knowledge" && (
              <span style={{ color: "var(--model-know)", marginRight: 6 }}>◆</span>
            )}
            {b.text}
          </div>
        ))}
      </div>
    );
  },
);
