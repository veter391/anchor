import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Card, type CardData } from "./Card";

// Phase 1: one hardcoded card proves the rendering treatment end to end.
// Later phases replace this with live match-engine state.
const HARDCODED: CardData = {
  title: "Why are you leaving your own company?",
  source: "prepared",
  bullets: [
    { text: "Good question, love building", state: "covered" },
    { text: "More depth + focus, strong team", state: "covered" },
    { text: "Project long-term, automated", state: "next" },
    { text: "Focus and energy into this role", state: "uncovered" },
    { text: "Impactful work matters", state: "uncovered" },
    { text: "Learn from seniors, grow", state: "uncovered" },
  ],
};

/** Reports the card's bounding box to Rust so the cursor-poll loop knows
 *  which regions are interactive; everything else stays click-through. */
function useInteractiveZone(ref: React.RefObject<HTMLDivElement | null>) {
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const report = () => {
      const r = el.getBoundingClientRect();
      invoke("set_interactive_zones", {
        zones: [{ x: r.x, y: r.y, w: r.width, h: r.height }],
      }).catch(() => {
        /* dev-server reload race; next report wins */
      });
    };
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
}

export function Overlay() {
  const cardRef = useRef<HTMLDivElement | null>(null);
  useInteractiveZone(cardRef);

  return (
    <div style={{ padding: 6 }}>
      <Card ref={cardRef} card={HARDCODED} />
    </div>
  );
}
