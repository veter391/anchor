import { forwardRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type BulletState = "covered" | "next" | "uncovered";
export type CardSource = "prepared" | "assembled" | "context" | "unexpected";

export interface CardData {
  title: string;
  source: CardSource;
  bullets: { text: string; state: BulletState; provenance?: "model_knowledge" }[];
}

const SOURCE_LABEL: Partial<Record<CardSource, { text: string; color: string }>> = {
  assembled: { text: "ASSEMBLED LIVE · from your material · ", color: "var(--assembled)" },
  context: { text: "CONTEXT · pre-flight research · ", color: "var(--text-muted)" },
  // Calm "heads up", not alarm. The ⚠ is small and amber, not red.
  unexpected: { text: "⚠ UNEXPECTED QUESTION · ", color: "var(--unexpected)" },
};

/** The overlay card. Glance-legibility rules from Documents/06_DESIGN.md:
 *  one card, max 6 bullets, 18px bullets, covered dims, next gets the accent
 *  bar, motion limited to colour shifts and a 120 ms card fade. */
export const Card = forwardRef<HTMLDivElement, { card: CardData }>(
  function Card({ card }, ref) {
    const hasModelKnowledge = card.bullets.some(
      (b) => b.provenance === "model_knowledge",
    );
    const allModelKnowledge =
      card.bullets.length > 0 &&
      card.bullets.every((b) => b.provenance === "model_knowledge");
    // The header must never claim "from your material" when nothing is.
    const label =
      card.source === "assembled" && allModelKnowledge
        ? { text: "ASSEMBLED LIVE · model knowledge · ", color: "var(--model-know)" }
        : SOURCE_LABEL[card.source];
    return (
      <div
        ref={ref}
        className="card-fade"
        style={{
          background: "var(--bg)",
          border: "1px solid var(--border)",
          borderTop:
            card.source === "assembled"
              ? "2px solid var(--assembled)"
              : card.source === "unexpected"
                ? "2px solid var(--unexpected)"
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
          {label && <span style={{ color: label.color }}>{label.text}</span>}
          {card.title}
        </div>

        {card.bullets.slice(0, 6).map((b, i) => (
          <div
            key={i}
            style={{
              fontSize: "var(--size-bullet)",
              lineHeight: "var(--leading)",
              // The NEXT anchor is the only bright-white line; later anchors
              // stay readable but a notch quieter; covered ones recede.
              color:
                b.state === "covered"
                  ? "var(--text-dim)"
                  : b.state === "next"
                    ? "var(--text)"
                    : "var(--text-soft)",
              background: b.state === "next" ? "var(--accent-bg)" : "transparent",
              borderLeft:
                b.state === "next"
                  ? "3px solid var(--accent)"
                  : "3px solid transparent",
              paddingLeft: 8,
              marginTop: i === 0 ? 0 : 7,
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

        {hasModelKnowledge && (
          <div
            style={{
              marginTop: 8,
              fontSize: "var(--size-meta)",
              color: "var(--model-know)",
            }}
          >
            ◆ contains model knowledge — not from your material
          </div>
        )}
      </div>
    );
  },
);
