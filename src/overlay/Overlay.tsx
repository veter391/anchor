import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
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

interface AssembledCard {
  title: string;
  source: "assembled" | "unexpected";
  bullets: { text: string; provenance: "assembled_grounded" | "model_knowledge" }[];
  has_model_knowledge: boolean;
}

function assembledToCardData(a: AssembledCard): CardData {
  return {
    title: a.title,
    source: a.source,
    bullets: a.bullets.map((b, i) => ({
      text: b.text,
      // Assembled cards have no coverage yet — first is "next".
      state: (i === 0 ? "next" : "uncovered") as BulletState,
      provenance: b.provenance === "model_knowledge" ? "model_knowledge" : undefined,
    })),
  };
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
      // Window sizing is owned by the single grow-to-fit effect below (which
      // enforces the 120-520 px bounds); this hook only reports click-through
      // zones, so it does not also resize (that fired a redundant second IPC and
      // let its no-floor path override the min height).
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
  // An assembled/panic card (Mode 2) overrides the prepared card until the
  // next confident jump. It carries no coverage of its own.
  const [assembled, setAssembled] = useState<CardData | null>(null);
  const [assembledKey, setAssembledKey] = useState(0);
  const rowIdRef = useRef<string | null>(null);
  const activeKey = assembled ? `asm-${assembledKey}` : (row?.id ?? "empty");
  useInteractiveZone(cardRef, activeKey);

  // Accent + card transparency come from the shared appearance settings and
  // are pushed live from Settings (appearance:changed). The overlay stays dark
  // regardless of the dashboard's light/dark theme (glance-legibility).
  useEffect(() => {
    const apply = (a: { accent?: string; overlay_opacity?: number }) => {
      const el = document.documentElement;
      if (a.accent) el.setAttribute("data-accent", a.accent);
      if (typeof a.overlay_opacity === "number") {
        const alpha = Math.max(0.4, Math.min(1, a.overlay_opacity / 100));
        el.style.setProperty("--card-alpha", String(alpha));
      }
    };
    invoke<{ accent: string; overlay_opacity: number }>("get_appearance")
      .then(apply)
      .catch(() => {});
    const un = listen<{ accent: string; overlay_opacity: number }>(
      "appearance:changed",
      (e) => apply(e.payload),
    );
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    const unShow = listen<CardRow>("card:show", (e) => {
      rowIdRef.current = e.payload.id;
      setRow(e.payload);
      setCovered([]);
      setAssembled(null); // a confident prepared jump takes over
    });
    const unCov = listen<CoverageUpdate>("coverage:update", (e) => {
      if (e.payload.card_id === rowIdRef.current) setCovered(e.payload.covered);
    });
    const unAsm = listen<AssembledCard>("card:assembled", (e) => {
      setAssembled(assembledToCardData(e.payload));
      setAssembledKey((k) => k + 1);
    });
    // Live loop reset / bound to a new session / stopped → back to waiting.
    const unClear = listen("live:cleared", () => {
      rowIdRef.current = null;
      setRow(null);
      setCovered([]);
      setAssembled(null);
    });
    return () => {
      unShow.then((f) => f()).catch(() => {});
      unCov.then((f) => f()).catch(() => {});
      unAsm.then((f) => f()).catch(() => {});
      unClear.then((f) => f()).catch(() => {});
    };
  }, []);

  // Grow the overlay window to fit the card. 06_DESIGN §3: a broad card may
  // carry up to 8 anchors and the overlay grows to a 520 px cap — there is
  // never a scrollbar. A fixed 300 px window silently clipped the 7th/8th
  // anchor off the bottom edge (drift audit 2026-07-28). Re-measured whenever
  // the card changes (`activeKey`) and via a ResizeObserver for content shifts.
  useEffect(() => {
    const el = cardRef.current;
    if (!el) return;
    const win = getCurrentWindow();
    const fit = () => {
      // +12 = the wrapper's 6 px top/bottom padding; width stays the card width.
      // Clamp 120-520: a 1-2 bullet card must not shrink below a glanceable
      // minimum, and a broad 8-anchor card caps at 520 with no scrollbar.
      const h = Math.max(120, Math.min(520, Math.ceil(el.getBoundingClientRect().height) + 12));
      void win.setSize(new LogicalSize(440, h)).catch(() => {});
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, [activeKey]);

  const card = assembled ?? (row ? withCoverage(row, covered) : EMPTY_STATE);

  return (
    <div style={{ padding: 6 }}>
      {/* key change restarts the 120 ms card fade */}
      <Card key={activeKey} ref={cardRef} card={card} />
    </div>
  );
}
