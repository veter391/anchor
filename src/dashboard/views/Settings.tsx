//! Settings — the ONE heavy tab (owner: everything weighty lives here so
//! every other screen stays light). Audio + Mode-2 engine + keys are the
//! everyday config; the transcript player, threshold sliders and live scores
//! are a Developer surface, off by default.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { LivePanel } from "../LivePanel";
import { panel, SectionTitle } from "../ui";

interface BootInfo {
  db_path: string;
  schema_version: number;
  embedding_model: string;
  embedding_dims: number;
}

export function Settings() {
  const [dev, setDev] = useState(false);
  const [boot, setBoot] = useState<BootInfo | null>(null);

  useEffect(() => {
    invoke<BootInfo>("boot_info").then(setBoot).catch(() => {});
  }, []);

  return (
    <div style={{ maxWidth: 920, margin: "0 auto", display: "grid", gap: 20 }}>
      <div>
        <h1 style={{ fontSize: 24, margin: "0 0 4px" }}>Settings</h1>
        <p style={{ color: "var(--text-muted)", fontSize: 14, margin: 0 }}>
          Audio, the unexpected-question engine, and where your data lives — all in one place.
        </p>
      </div>

      <LivePanel developer={dev} />

      <section style={panel}>
        <SectionTitle>Data</SectionTitle>
        <div style={{ fontSize: 13, color: "var(--text-soft)", display: "grid", gap: 4 }}>
          <div>
            Everything is stored locally in one SQLite file. Audio is never written to disk.
          </div>
          {boot && (
            <>
              <div style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)", fontSize: 12 }}>
                {boot.db_path}
              </div>
              <div style={{ color: "var(--text-dim)", fontSize: 12 }}>
                schema v{boot.schema_version} · embeddings {boot.embedding_model} (
                {boot.embedding_dims}d)
              </div>
            </>
          )}
        </div>
      </section>

      <section style={panel}>
        <label style={{ display: "flex", alignItems: "center", gap: 10, cursor: "pointer" }}>
          <input type="checkbox" checked={dev} onChange={(e) => setDev(e.target.checked)} />
          <span>
            <span style={{ fontSize: 14 }}>Developer tools</span>
            <span style={{ display: "block", fontSize: 12, color: "var(--text-muted)" }}>
              Transcript player, live match scores, threshold sliders. Off for everyday use.
            </span>
          </span>
        </label>
      </section>
    </div>
  );
}
