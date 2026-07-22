import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Card, type CardData, type BulletState, type CardSource } from "./Card";

interface CardRow {
  id: string;
  title: string;
  source: string;
  bullets: string[];
}

interface CoverageUpdate {
  card_id: string;
  covered: boolean[];
}

const EMPTY_STATE: CardData = {
  title: "Anchor — waiting for a card",
  source: "prepared",
  bullets: [
    { text: "Import cards in the dashboard", state: "next" },
    { text: "Feed or play a transcript", state: "uncovered" },
    { text: "The right card appears here", state: "uncovered" },
  ],
};

/** Coverage → visual states: covered dims, first uncovered is NEXT. */
function withCoverage(row: CardRow, covered: boolean[]): CardData {
  let nextAssigned = false;
  return {
    title: row.title,
    source: (row.source as CardSource) ?? "prepared",
    bullets: row.bullets.map((text, i) => {
      let state: BulletState;
      if (covered[i]) {
        state = "covered";
      } else if (!nextAssigned) {
        state = "next";
        nextAssigned = true;
      } else {
        state = "uncovered";
      }
      return { text, state };
    }),
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
  const [row, setRow] = useState<CardRow | null>(null);
  const [covered, setCovered] = useState<boolean[]>([]);
  const rowIdRef = useRef<string | null>(null);
  useInteractiveZone(cardRef, row?.id ?? "empty");

  useEffect(() => {
    const unShow = listen<CardRow>("card:show", (e) => {
      rowIdRef.current = e.payload.id;
      setRow(e.payload);
      setCovered([]);
    });
    // A coverage event can race a jump inside one tick; the payload's
    // card_id makes sure stale coverage never lands on the wrong card.
    const unCov = listen<CoverageUpdate>("coverage:update", (e) => {
      if (e.payload.card_id === rowIdRef.current) {
        setCovered(e.payload.covered);
      }
    });
    return () => {
      unShow.then((f) => f()).catch(() => {});
      unCov.then((f) => f()).catch(() => {});
    };
  }, []);

  const card = row ? withCoverage(row, covered) : EMPTY_STATE;

  return (
    <div style={{ padding: 6 }}>
      {/* key change restarts the 120 ms card fade */}
      <Card key={row?.id ?? "empty"} ref={cardRef} card={card} />
    </div>
  );
}
