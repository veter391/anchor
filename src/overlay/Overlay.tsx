import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Card, type CardData, type BulletState } from "./Card";

interface CardRow {
  id: string;
  title: string;
  source: string;
  bullets: string[];
}

const EMPTY_STATE: CardData = {
  title: "Anchor — waiting for a card",
  source: "prepared",
  bullets: [
    { text: "Import cards in the dashboard", state: "next" },
    { text: "Type a question in the query box", state: "uncovered" },
    { text: "The right card appears here", state: "uncovered" },
  ],
};

function toCardData(row: CardRow): CardData {
  return {
    title: row.title,
    source: (row.source as CardData["source"]) ?? "prepared",
    bullets: row.bullets.map((text, i) => ({
      text,
      // Coverage arrives in Phase 3; until then the first bullet is "next".
      state: (i === 0 ? "next" : "uncovered") as BulletState,
    })),
  };
}

/** Reports the card's bounding box to Rust so the cursor-poll loop knows
 *  which regions are interactive, and asks the window to fit the content
 *  height so wrapping bullets never clip. Zones are CSS/logical px relative
 *  to the viewport — Rust converts the cursor into the same space per tick,
 *  so drags and DPI scaling need no re-report. Re-runs on every card swap. */
function useInteractiveZone(
  ref: React.RefObject<HTMLDivElement | null>,
  cardKey: string,
) {
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
      invoke("fit_overlay_height", { height: r.bottom + 8 }).catch(() => {});
    };
    report();
    const ro = new ResizeObserver(report);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref, cardKey]);
}

export function Overlay() {
  const cardRef = useRef<HTMLDivElement | null>(null);
  const [card, setCard] = useState<CardData>(EMPTY_STATE);
  useInteractiveZone(cardRef, card.title);

  useEffect(() => {
    const un = listen<CardRow>("card:show", (e) => setCard(toCardData(e.payload)));
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  return (
    <div style={{ padding: 6 }}>
      {/* key change restarts the 120 ms card fade */}
      <Card key={card.title} ref={cardRef} card={card} />
    </div>
  );
}
