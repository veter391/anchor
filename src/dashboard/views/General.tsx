//! General — the friendly landing. A first-time, non-technical user opens
//! Anchor here and gets it: what it is, the learn-first idea, where to click.
//! Warm and editorial, not a stack of identical boxes.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { panel, btn, Mark, type NavKey } from "../ui";

interface BootInfo {
  cards: number;
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
    emoji: "📚",
    title: "Learn",
    body: "Anchor is useless until you study. Take the study prompt to whatever AI you learn with — and actually learn it.",
    action: "Open study prompt",
    go: "cards" as NavKey,
  },
  {
    n: "2",
    emoji: "✍️",
    title: "Build cards",
    body: "Turn what you know into anchor cards — six keywords, never sentences. Or drop in raw notes and let Anchor draft them.",
    action: "Go to cards",
    go: "cards" as NavKey,
  },
  {
    n: "3",
    emoji: "🎧",
    title: "Load & go",
    body: "Make a session, drop your cards in, take the call. The right card appears the moment the topic comes up.",
    action: "New session",
    go: "sessions" as NavKey,
  },
];

const HOW = [
  "It listens to both sides of the call, locally.",
  "It shows your right card the moment the topic comes up.",
  "It tracks which points you have already covered.",
  "Nothing ever leaves your machine.",
];

export function General({ onNavigate }: { onNavigate: (k: NavKey) => void }) {
  const [boot, setBoot] = useState<BootInfo | null>(null);
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  // No speech model installed → the user would only discover the mandatory
  // download when "Go live" fails. Surface it here, up front.
  const [needsModel, setNeedsModel] = useState(false);

  useEffect(() => {
    invoke<BootInfo>("boot_info").then(setBoot).catch(() => {});
    invoke<SessionRow[]>("list_sessions")
      .then((s) => setSessions(s.filter((x) => x.status !== "archived")))
      .catch(() => {});
    invoke<{ installed: boolean }[]>("list_asr_models")
      .then((rows) => setNeedsModel(rows.length > 0 && rows.every((r) => !r.installed)))
      .catch(() => {});
  }, []);

  return (
    <div style={{ maxWidth: 940, margin: "0 auto", display: "grid", gap: 40 }}>
      {/* Hero — borderless, breathing */}
      <header style={{ paddingTop: 6 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10, color: "var(--text-muted)", fontSize: 14 }}>
          <Mark size={22} />
          <span>Welcome back 👋</span>
        </div>
        <h1 style={{ fontSize: 40, lineHeight: 1.08, margin: "18px 0 10px", letterSpacing: "-0.02em" }}>
          The right six words,
          <br />
          <span style={{ color: "var(--accent)" }}>the moment you need them.</span>
        </h1>
        <p style={{ color: "var(--text-soft)", fontSize: 17, margin: 0, maxWidth: 600, lineHeight: 1.5 }}>
          A live notes overlay for your calls. You prepare your own cards; Anchor surfaces the
          right one as the topic comes up, and tracks what you have covered.
        </p>
      </header>

      {/* First-run setup nudge — only until a speech model is installed */}
      {needsModel && (
        <section
          style={{
            ...panel,
            borderColor: "var(--assembled)",
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 16,
            flexWrap: "wrap",
          }}
        >
          <div>
            <div style={{ fontSize: 15, fontWeight: 600 }}>🎙️ One setup step: download a speech model</div>
            <p style={{ color: "var(--text-muted)", fontSize: 13.5, margin: "4px 0 0", maxWidth: 560, lineHeight: 1.5 }}>
              Anchor needs an on-device speech model to hear your calls — about 680 MB, downloaded
              once, then cached offline. Grab one before your first session.
            </p>
          </div>
          <button
            className="press"
            onClick={() => onNavigate("settings")}
            style={{ ...btn, borderColor: "var(--assembled)", color: "var(--assembled)" }}
          >
            Download a model
          </button>
        </section>
      )}

      {/* Three steps — light interactive cards, mixed with the hero above */}
      <section>
        <div style={{ color: "var(--text-muted)", fontSize: 12, letterSpacing: "0.09em", textTransform: "uppercase", marginBottom: 14 }}>
          Three steps to ready
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(250px, 1fr))", gap: 16 }}>
          {STEPS.map((s) => (
            <button
              key={s.n}
              className="lift press"
              onClick={() => onNavigate(s.go)}
              style={{
                ...panel,
                textAlign: "left",
                cursor: "pointer",
                display: "flex",
                flexDirection: "column",
                gap: 10,
                color: "var(--text)",
              }}
            >
              <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
                <span
                  style={{
                    width: 32,
                    height: 32,
                    borderRadius: 9,
                    background: "var(--accent-bg)",
                    color: "var(--accent)",
                    display: "grid",
                    placeItems: "center",
                    fontWeight: 700,
                    fontSize: 15,
                  }}
                >
                  {s.n}
                </span>
                <span aria-hidden style={{ fontSize: 20, opacity: 0.9 }}>
                  {s.emoji}
                </span>
              </div>
              <div style={{ fontSize: 18, fontWeight: 600 }}>{s.title}</div>
              <p style={{ color: "var(--text-muted)", fontSize: 13.5, margin: 0, flex: 1, lineHeight: 1.5 }}>
                {s.body}
              </p>
              <span className="link" style={{ fontSize: 13.5, fontWeight: 500 }}>
                {s.action} →
              </span>
            </button>
          ))}
        </div>
      </section>

      {/* How it works — borderless editorial strip (breaks the box rhythm) */}
      <section>
        <div style={{ color: "var(--text-muted)", fontSize: 12, letterSpacing: "0.09em", textTransform: "uppercase", marginBottom: 16 }}>
          How it works, in 20 seconds
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(210px, 1fr))", gap: 20 }}>
          {HOW.map((h, i) => (
            <div key={i} style={{ display: "flex", gap: 12, alignItems: "start" }}>
              <span style={{ color: "var(--accent)", fontWeight: 700, fontSize: 20, lineHeight: 1 }}>
                {i + 1}
              </span>
              <span style={{ color: "var(--text-soft)", fontSize: 14.5, lineHeight: 1.45 }}>{h}</span>
            </div>
          ))}
        </div>
      </section>

      {/* Sessions */}
      <section>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 14 }}>
          <div style={{ color: "var(--text-muted)", fontSize: 12, letterSpacing: "0.09em", textTransform: "uppercase" }}>
            {sessions.length ? "Jump back in" : "Your sessions"}
          </div>
          <button className="press" onClick={() => onNavigate("sessions")} style={btn}>
            {sessions.length ? "All sessions" : "New session"}
          </button>
        </div>
        {sessions.length ? (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(230px, 1fr))", gap: 12 }}>
            {sessions.slice(0, 6).map((s) => (
              <button
                key={s.id}
                className="lift"
                onClick={() => onNavigate("sessions")}
                style={{ ...panel, textAlign: "left", cursor: "pointer", display: "grid", gap: 6, color: "var(--text)" }}
              >
                <span style={{ fontSize: 15 }}>{s.title}</span>
                <span style={{ fontSize: 12, color: "var(--text-muted)" }}>{s.kind}</span>
              </button>
            ))}
          </div>
        ) : (
          <p style={{ color: "var(--text-muted)", fontSize: 14.5, margin: 0, lineHeight: 1.55, maxWidth: 620 }}>
            No sessions yet. A session is one call — its cards, its transcript, its coverage
            report. Create one when you are ready.
            {boot ? (
              <span style={{ color: "var(--text-dim)" }}> You have {boot.cards} cards in your library.</span>
            ) : null}
          </p>
        )}
      </section>
    </div>
  );
}
