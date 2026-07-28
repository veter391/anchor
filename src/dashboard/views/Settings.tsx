//! Settings — the ONE heavy tab, but written for a normal person: plain
//! language, no jargon on the surface. Appearance, the unexpected-question
//! engine, and where your data lives. The transcript player / live scores /
//! thresholds are a Developer surface, off by default.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LivePanel } from "../LivePanel";
import { Mode2Settings } from "../Mode2Settings";
import { AsrModels } from "../AsrModels";
import { panel, SectionTitle, applyAppearance } from "../ui";

interface BootInfo {
  db_path: string;
  schema_version: number;
  embedding_model: string;
  embedding_dims: number;
}
interface Appearance {
  accent: string;
  theme: string;
  overlay_opacity: number;
}

const ACCENTS = [
  { key: "teal", label: "Teal", swatch: "#4fd1c5" },
  { key: "coral", label: "Coral", swatch: "#ee889a" },
  { key: "amber", label: "Amber", swatch: "#eab04a" },
];

export function Settings() {
  const [dev, setDev] = useState(false);
  const [boot, setBoot] = useState<BootInfo | null>(null);
  const [look, setLook] = useState<Appearance>({ accent: "teal", theme: "dark", overlay_opacity: 90 });
  const [captureExcluded, setCaptureExcluded] = useState(false);
  const [capErr, setCapErr] = useState<string | null>(null);
  const [asrEngine, setAsrEngine] = useState("auto");

  useEffect(() => {
    invoke<BootInfo>("boot_info").then(setBoot).catch(() => {});
    invoke<Appearance>("get_appearance").then(setLook).catch(() => {});
    invoke<boolean>("get_capture_excluded").then(setCaptureExcluded).catch(() => {});
    invoke<string>("get_asr_engine").then(setAsrEngine).catch(() => {});
  }, []);

  const changeEngine = (engine: string) => {
    setAsrEngine(engine);
    invoke("set_asr_engine", { engine }).catch(() => {});
  };

  const toggleCapture = async (on: boolean) => {
    setCaptureExcluded(on);
    setCapErr(null);
    try {
      await invoke("set_capture_excluded", { on });
    } catch (e) {
      // Protection did NOT change — never leave the box implying it did, or the
      // user shares their screen believing notes are hidden when they are not.
      setCaptureExcluded(!on);
      setCapErr(`Could not change screen-share protection: ${String(e)}`);
    }
  };

  const save = (patch: Partial<Appearance>) => {
    const next = { ...look, ...patch };
    setLook(next);
    applyAppearance(next); // instant on the dashboard
    invoke("set_appearance", {
      accent: next.accent,
      theme: next.theme,
      overlayOpacity: next.overlay_opacity,
    }).catch(() => {});
  };

  return (
    <div style={{ maxWidth: 860, margin: "0 auto", display: "grid", gap: 22 }}>
      <div>
        <h1 style={{ fontSize: 26, margin: "0 0 4px", letterSpacing: "-0.01em" }}>Settings</h1>
        <p style={{ color: "var(--text-muted)", fontSize: 14.5, margin: 0 }}>
          Make it yours, and choose how the unexpected-question help runs.
        </p>
      </div>

      {/* Appearance */}
      <section style={panel}>
        <SectionTitle emoji="🎨" hint="Anchor keeps the same calm dark look — pick the colour and mood that suits you.">
          Appearance
        </SectionTitle>

        <div style={{ display: "grid", gap: 16 }}>
          <div>
            <div style={{ fontSize: 13.5, marginBottom: 8, color: "var(--text-soft)" }}>Accent colour</div>
            <div style={{ display: "flex", gap: 10 }}>
              {ACCENTS.map((a) => {
                const on = look.accent === a.key;
                return (
                  <button
                    key={a.key}
                    className="press"
                    onClick={() => save({ accent: a.key })}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: 8,
                      padding: "8px 12px",
                      borderRadius: 10,
                      cursor: "pointer",
                      background: on ? "var(--accent-bg)" : "transparent",
                      border: `1px solid ${on ? "var(--accent)" : "var(--border)"}`,
                      color: on ? "var(--accent)" : "var(--text-muted)",
                    }}
                  >
                    <span style={{ width: 14, height: 14, borderRadius: "50%", background: a.swatch }} />
                    {a.label}
                  </button>
                );
              })}
            </div>
          </div>

          <div>
            <div style={{ fontSize: 13.5, marginBottom: 8, color: "var(--text-soft)" }}>Theme</div>
            <div style={{ display: "flex", gap: 10 }}>
              {["dark", "light"].map((t) => {
                const on = look.theme === t;
                return (
                  <button
                    key={t}
                    className="press"
                    onClick={() => save({ theme: t })}
                    style={{
                      padding: "8px 16px",
                      borderRadius: 10,
                      cursor: "pointer",
                      textTransform: "capitalize",
                      background: on ? "var(--accent-bg)" : "transparent",
                      border: `1px solid ${on ? "var(--accent)" : "var(--border)"}`,
                      color: on ? "var(--accent)" : "var(--text-muted)",
                    }}
                  >
                    {t === "dark" ? "🌙 Dark" : "☀️ Light"}
                  </button>
                );
              })}
            </div>
          </div>

          <div>
            <div style={{ fontSize: 13.5, marginBottom: 4, color: "var(--text-soft)" }}>
              Card transparency{" "}
              <span style={{ color: "var(--text-dim)" }}>
                {look.overlay_opacity === 100 ? "solid" : `${look.overlay_opacity}%`}
              </span>
            </div>
            <p style={{ margin: "0 0 8px", fontSize: 12.5, color: "var(--text-muted)" }}>
              How see-through the floating card is — turn it down when you are sharing your whole
              screen and want the desktop to show through.
            </p>
            <input
              type="range"
              min={40}
              max={100}
              step={5}
              value={look.overlay_opacity}
              onChange={(e) => save({ overlay_opacity: Number(e.target.value) })}
              style={{ width: 280, accentColor: "var(--accent)" }}
            />
          </div>
        </div>
      </section>

      {/* Unexpected-question engine */}
      <section style={panel}>
        <SectionTitle emoji="💡" hint="When a question you did not prepare comes up, Anchor builds a card from your material. Run it free on your machine, or bring your own API key.">
          When a question catches you off guard
        </SectionTitle>
        <Mode2Settings />
      </section>

      {/* Data */}
      <section style={panel}>
        <SectionTitle emoji="🔒">Your data</SectionTitle>
        <div style={{ fontSize: 13.5, color: "var(--text-soft)", display: "grid", gap: 6 }}>
          <div>Everything stays on your machine, in one file. Audio is never written to disk.</div>
          {boot && (
            <div style={{ color: "var(--text-dim)", fontSize: 12, fontFamily: "var(--font-mono)" }}>
              {boot.db_path}
            </div>
          )}
        </div>
      </section>

      {/* Screen sharing */}
      <section style={panel}>
        <SectionTitle emoji="🖥️">Screen sharing</SectionTitle>
        <label style={{ display: "flex", alignItems: "flex-start", gap: 10, cursor: "pointer" }}>
          <input
            type="checkbox"
            checked={captureExcluded}
            onChange={(e) => toggleCapture(e.target.checked)}
            style={{ marginTop: 3 }}
          />
          <span>
            <span style={{ fontSize: 14 }}>Hide my notes when I share my screen</span>
            <span style={{ display: "block", fontSize: 12.5, color: "var(--text-muted)", lineHeight: 1.5 }}>
              Off by default — Anchor has no stealth mode. When on, the floating card is kept out
              of a Teams/Zoom/OBS share (and out of your own recordings). During a live call you can
              reveal it any time with “Show notes”.
            </span>
          </span>
        </label>
        {capErr && (
          <div style={{ marginTop: 10, fontSize: 12.5, color: "var(--red)" }}>{capErr}</div>
        )}
      </section>

      {/* Speech recognition model */}
      <section style={{ ...panel, display: "grid", gap: 10 }}>
        <SectionTitle emoji="🎙️" hint="The on-device model that turns speech into text. Automatic suits almost everyone. Pick English if all your calls are in English — it's the fastest. Pick Compatibility if a slower computer struggles to keep up.">
          Speech model
        </SectionTitle>
        <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
          <select
            value={asrEngine}
            onChange={(e) => changeEngine(e.target.value)}
            style={{
              background: "var(--bg-elevated)",
              color: "var(--text)",
              border: "1px solid var(--border)",
              borderRadius: 8,
              padding: "9px 12px",
              fontSize: 14,
            }}
          >
            <option value="auto">Automatic (recommended)</option>
            <option value="multilingual">Multilingual — English, Spanish, Russian, Ukrainian, German</option>
            <option value="english">English only — fastest</option>
            <option value="offline">Compatibility — for slower computers</option>
          </select>
          <span style={{ fontSize: 12.5, color: "var(--text-muted)" }}>
            Takes effect on your next call.
          </span>
        </div>
        <AsrModels />
      </section>

      {/* Developer */}
      <section style={panel}>
        <label style={{ display: "flex", alignItems: "center", gap: 10, cursor: "pointer" }}>
          <input type="checkbox" checked={dev} onChange={(e) => setDev(e.target.checked)} />
          <span>
            <span style={{ fontSize: 14 }}>Developer tools</span>
            <span style={{ display: "block", fontSize: 12.5, color: "var(--text-muted)" }}>
              Transcript player, live match scores, threshold sliders, the ASR model and embedding
              details. Off for everyday use.
            </span>
          </span>
        </label>
      </section>

      {dev && <LivePanel />}
    </div>
  );
}
