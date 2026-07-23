//! General — the friendly landing. A first-time, non-technical user opens
//! Anchor here and understands what it is, the learn-first idea, and where to
//! click. No terminal, no debug (06_DESIGN / 09_PLAN Phase 6 owner vision).

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { panel, btn, btnGhost, Wordmark, type NavKey } from "../ui";

interface BootInfo {
  cards: number;
  embedder_loaded: boolean;
}
interface SessionRow {
  id: string;
  title: string;
  kind: string;
  status: string;
}

const STEPS = [
  {
    n: "1",
    title: "Learn",
    body: "Anchor is useless until you study. Take the study prompt to whatever AI you learn with and actually learn the material.",
    action: "Open study prompt",
    go: "cards" as NavKey,
  },
  {
    n: "2",
    title: "Build cards",
    body: "Turn what you now know into anchor cards — six keywords, not sentences. Or drop in raw notes and let Anchor draft them.",
    action: "Go to cards",
    go: "cards" as NavKey,
  },
  {
    n: "3",
    title: "Load & go",
    body: "Make a session, drop your cards in, check both audio channels, take the call. The right card appears the moment the topic comes up.",
    action: "New session",
    go: "sessions" as NavKey,
  },
];

const HOW = [
  "It listens to both sides of the call, locally.",
  "It shows your right card the moment the topic comes up.",
  "It tracks which points you have already covered.",
  "Nothing leaves your machine.",
];

export function General({ onNavigate }: { onNavigate: (k: NavKey) => void }) {
  const [boot, setBoot] = useState<BootInfo | null>(null);
  const [sessions, setSessions] = useState<SessionRow[]>([]);

  useEffect(() => {
    invoke<BootInfo>("boot_info").then(setBoot).catch(() => {});
    invoke<SessionRow[]>("list_sessions")
      .then((s) => setSessions(s.filter((x) => x.status !== "archived")))
      .catch(() => {});
  }, []);

  return (
    <div style={{ maxWidth: 920, margin: "0 auto", display: "grid", gap: 28 }}>
      <header style={{ marginTop: 8 }}>
        <Wordmark size={26} />
        <h1 style={{ fontSize: 30, margin: "14px 0 6px", lineHeight: 1.15 }}>
          The right six words,
          <br />
          in front of your eyes.
        </h1>
        <p style={{ color: "var(--text-muted)", fontSize: 16, margin: 0, maxWidth: 620 }}>
          A live notes overlay for your calls. You prepare your own cards; Anchor puts the right
          one in front of you the moment the topic comes up — and tracks what you have covered.
          <span style={{ color: "var(--accent)" }}> Prepared, not prompted.</span>
        </p>
      </header>

      {/* The three steps */}
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fit, minmax(240px, 1fr))",
          gap: 16,
        }}
      >
        {STEPS.map((s) => (
          <div
            key={s.n}
            style={{
              ...panel,
              display: "flex",
              flexDirection: "column",
              gap: 10,
            }}
          >
            <div
              style={{
                width: 34,
                height: 34,
                borderRadius: 9,
                background: "var(--accent-bg)",
                color: "var(--accent)",
                display: "grid",
                placeItems: "center",
                fontWeight: 700,
                fontSize: 16,
              }}
            >
              {s.n}
            </div>
            <div style={{ fontSize: 17, fontWeight: 600 }}>{s.title}</div>
            <p style={{ color: "var(--text-muted)", fontSize: 13.5, margin: 0, flex: 1 }}>
              {s.body}
            </p>
            <button
              onClick={() => onNavigate(s.go)}
              style={{ ...btnGhost, alignSelf: "start" }}
            >
              {s.action}
            </button>
          </div>
        ))}
      </div>

      {/* How it works in 20 seconds */}
      <section style={{ ...panel, background: "var(--bg-elevated)" }}>
        <h3
          style={{
            margin: "0 0 12px",
            fontSize: 12,
            letterSpacing: "0.08em",
            textTransform: "uppercase",
            color: "var(--text-muted)",
          }}
        >
          How it works, in 20 seconds
        </h3>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
            gap: 14,
          }}
        >
          {HOW.map((h, i) => (
            <div key={i} style={{ display: "flex", gap: 10, alignItems: "start" }}>
              <span style={{ color: "var(--accent)", fontWeight: 700 }}>{i + 1}</span>
              <span style={{ color: "var(--text-soft)", fontSize: 14 }}>{h}</span>
            </div>
          ))}
        </div>
      </section>

      {/* Jump back in / empty state */}
      <section>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            marginBottom: 12,
          }}
        >
          <h3
            style={{
              margin: 0,
              fontSize: 12,
              letterSpacing: "0.08em",
              textTransform: "uppercase",
              color: "var(--text-muted)",
            }}
          >
            {sessions.length ? "Jump back in" : "Your sessions"}
          </h3>
          <button onClick={() => onNavigate("sessions")} style={btn}>
            {sessions.length ? "All sessions" : "New session"}
          </button>
        </div>
        {sessions.length ? (
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(220px, 1fr))",
              gap: 12,
            }}
          >
            {sessions.slice(0, 6).map((s) => (
              <button
                key={s.id}
                onClick={() => onNavigate("sessions")}
                style={{
                  ...panel,
                  textAlign: "left",
                  cursor: "pointer",
                  display: "grid",
                  gap: 6,
                }}
              >
                <span style={{ fontSize: 15, color: "var(--text)" }}>{s.title}</span>
                <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{s.kind}</span>
              </button>
            ))}
          </div>
        ) : (
          <div style={{ ...panel, color: "var(--text-muted)", fontSize: 14 }}>
            No sessions yet. A session is one call — its cards, its transcript, its coverage
            report. Create one when you are ready to take a call.
            <div style={{ marginTop: 6, color: "var(--text-dim)", fontSize: 13 }}>
              {boot ? `${boot.cards} cards in your library.` : ""}
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
