import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Thresholds {
  theta_card: number;
  theta_margin: number;
  stable_ticks: number;
  cooldown_ms: number;
  theta_bullet: number;
}

interface DebugCandidate {
  card_id: string;
  title: string;
  score: number;
  vec_rank: number | null;
  bm25_rank: number | null;
  vec_cos: number | null;
}

interface DebugState {
  them_text: string;
  me_text: string;
  top: DebugCandidate[];
  thresholds: Thresholds;
  current_card: string | null;
  challenger: [string, number] | null;
  no_confidence: boolean;
  bullet_sims: number[];
  tick_ms: number;
}

const panel: React.CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border)",
  borderRadius: 8,
  padding: 14,
};

const mono: React.CSSProperties = {
  fontFamily: "var(--font-mono)",
  fontSize: 12,
};

const SAMPLE = `them: so tell me why are you leaving your own company
me: good question, I love building things
me: but I want more depth and focus with a strong team
them: okay and how did the kubernetes migration go
me: we moved forty services with zero downtime
me: we used helm and argocd, full gitops
them: what about money, what are your salary expectations
me: I researched the market range and I'm flexible for the right role`;

/** Speaking pace: ~2.5 words/second, min 700 ms per line. */
function lineDelayMs(line: string): number {
  const words = line.split(/\s+/).length;
  return Math.max(700, (words / 2.5) * 1000);
}

export function LivePanel() {
  const [script, setScript] = useState(SAMPLE);
  const [playing, setPlaying] = useState(false);
  const [progress, setProgress] = useState<string | null>(null);
  const [debug, setDebug] = useState<DebugState | null>(null);
  const [th, setTh] = useState<Thresholds | null>(null);
  const stopRef = useRef(false);

  useEffect(() => {
    invoke<Thresholds>("get_thresholds").then(setTh).catch(() => {});
    const un = listen<DebugState>("match:debug", (e) => setDebug(e.payload));
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  const play = async () => {
    setPlaying(true);
    stopRef.current = false;
    await invoke("reset_live").catch(() => {});
    const lines = script
      .split("\n")
      .map((l) => l.trim())
      .filter(Boolean);
    for (let i = 0; i < lines.length; i++) {
      if (stopRef.current) break;
      const line = lines[i]!;
      const m = /^(them|me)\s*:\s*(.+)$/i.exec(line);
      if (!m) continue;
      setProgress(`${i + 1}/${lines.length} · ${line.slice(0, 60)}`);
      await invoke("feed_transcript", {
        speaker: m[1]!.toLowerCase(),
        text: m[2]!,
      }).catch(() => {});
      await new Promise((r) => setTimeout(r, lineDelayMs(m[2]!)));
    }
    setPlaying(false);
    setProgress(null);
  };

  const applyTh = (patch: Partial<Thresholds>) => {
    if (!th) return;
    const next = { ...th, ...patch };
    setTh(next);
    invoke("set_thresholds", { thresholds: next }).catch(() => {});
  };

  return (
    <>
      <section style={panel}>
        <h3 style={{ margin: "0 0 8px", fontSize: 13, color: "var(--text-muted)" }}>
          TRANSCRIPT PLAYER — Phase 3 (them: / me: lines, played at speaking pace)
        </h3>
        <textarea
          value={script}
          onChange={(e) => setScript(e.target.value)}
          rows={6}
          style={{
            width: "100%",
            boxSizing: "border-box",
            background: "var(--bg-elevated)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            color: "var(--text)",
            padding: 10,
            ...mono,
            fontSize: 13,
          }}
        />
        <div style={{ marginTop: 8, display: "flex", gap: 8, alignItems: "center" }}>
          {!playing ? (
            <button onClick={play} style={btn}>
              ▶ Play
            </button>
          ) : (
            <button
              onClick={() => {
                stopRef.current = true;
              }}
              style={{ ...btn, borderColor: "var(--red)", color: "var(--red)" }}
            >
              ■ Stop
            </button>
          )}
          <button onClick={() => invoke("reset_live").catch(() => {})} style={btn}>
            Reset session
          </button>
          {progress && (
            <span style={{ color: "var(--text-muted)", fontSize: 12 }}>{progress}</span>
          )}
        </div>
      </section>

      <section style={panel}>
        <h3 style={{ margin: "0 0 8px", fontSize: 13, color: "var(--text-muted)" }}>
          DEBUG — the thresholds are the product
        </h3>
        {th && (
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fit, minmax(170px, 1fr))",
              gap: 10,
              marginBottom: 12,
            }}
          >
            <Slider
              label={`θ_card ${th.theta_card.toFixed(2)}`}
              min={0}
              max={1}
              step={0.01}
              value={th.theta_card}
              onChange={(v) => applyTh({ theta_card: v })}
            />
            <Slider
              label={`θ_margin ${th.theta_margin.toFixed(2)}`}
              min={0}
              max={0.5}
              step={0.01}
              value={th.theta_margin}
              onChange={(v) => applyTh({ theta_margin: v })}
            />
            <Slider
              label={`stable ticks ${th.stable_ticks}`}
              min={1}
              max={5}
              step={1}
              value={th.stable_ticks}
              onChange={(v) => applyTh({ stable_ticks: Math.round(v) })}
            />
            <Slider
              label={`cooldown ${th.cooldown_ms} ms`}
              min={0}
              max={5000}
              step={100}
              value={th.cooldown_ms}
              onChange={(v) => applyTh({ cooldown_ms: Math.round(v) })}
            />
            <Slider
              label={`θ_bullet ${th.theta_bullet.toFixed(2)}`}
              min={0}
              max={1}
              step={0.01}
              value={th.theta_bullet}
              onChange={(v) => applyTh({ theta_bullet: v })}
            />
          </div>
        )}
        {debug ? (
          <div style={{ display: "grid", gap: 8 }}>
            <div style={mono}>
              <span style={{ color: "var(--text-muted)" }}>them ▸ </span>
              <span style={{ color: "var(--text-soft)" }}>{debug.them_text || "—"}</span>
            </div>
            <div style={mono}>
              <span style={{ color: "var(--text-muted)" }}>me &nbsp;&nbsp;▸ </span>
              <span style={{ color: "var(--text-soft)" }}>{debug.me_text || "—"}</span>
            </div>
            <table style={{ ...mono, borderSpacing: "0 2px" }}>
              <tbody>
                {debug.top.map((t, i) => (
                  <tr
                    key={t.card_id}
                    style={{
                      color:
                        t.card_id === debug.current_card
                          ? "var(--accent)"
                          : i === 0
                            ? "var(--text)"
                            : "var(--text-muted)",
                    }}
                  >
                    <td style={{ paddingRight: 10 }}>
                      {t.card_id === debug.current_card ? "●" : `#${i + 1}`}
                    </td>
                    <td style={{ paddingRight: 10 }}>{t.title.slice(0, 44)}</td>
                    <td style={{ paddingRight: 10 }}>score {t.score.toFixed(3)}</td>
                    <td style={{ paddingRight: 10 }}>
                      cos {t.vec_cos != null ? t.vec_cos.toFixed(3) : "–"}
                    </td>
                    <td>
                      vec {t.vec_rank ?? "–"} · bm25 {t.bm25_rank ?? "–"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div style={{ ...mono, color: "var(--text-muted)" }}>
              {debug.no_confidence && (
                <span style={{ color: "var(--assembled)" }}>below θ_card (Mode-2 territory) · </span>
              )}
              {debug.challenger && (
                <span>
                  challenger {debug.challenger[0].slice(0, 8)}… streak {debug.challenger[1]} ·{" "}
                </span>
              )}
              bullets [{debug.bullet_sims.map((s) => s.toFixed(2)).join(" ")}] · tick{" "}
              {debug.tick_ms} ms
            </div>
          </div>
        ) : (
          <div style={{ color: "var(--text-dim)", fontSize: 13 }}>
            Feed or play a transcript to see live scores.
          </div>
        )}
      </section>
    </>
  );
}

function Slider(props: {
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <label style={{ fontSize: 12, color: "var(--text-muted)" }}>
      {props.label}
      <input
        type="range"
        min={props.min}
        max={props.max}
        step={props.step}
        value={props.value}
        onChange={(e) => props.onChange(Number(e.target.value))}
        style={{ width: "100%", accentColor: "var(--accent)" }}
      />
    </label>
  );
}

const btn: React.CSSProperties = {
  background: "var(--bg)",
  color: "var(--accent)",
  border: "1px solid var(--accent)",
  borderRadius: 6,
  padding: "8px 18px",
  fontSize: 14,
  cursor: "pointer",
};
