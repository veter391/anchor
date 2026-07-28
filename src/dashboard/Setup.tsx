//! First-run setup. After consent, if no speech model is installed yet, Anchor
//! downloads the default one automatically — the user does nothing and picks
//! nothing. One progress bar, then the app is ready. (Advanced users can add or
//! swap models later in Settings.) The embedding model downloads in the
//! background at startup; this screen tracks the larger speech model.

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Mark } from "./ui";

const DEFAULT_MODEL = "multilingual";

// The auto-download must fire EXACTLY ONCE per app session. A second concurrent
// call is rejected by the backend's in-flight guard ("already downloading") and
// would flash a spurious error over a perfectly healthy download. This module
// flag makes the kick-off idempotent regardless of how many times the effect
// runs or the component remounts; the Retry button below bypasses it on purpose.
let autoStarted = false;

function beginDownload(onErr: (e: string) => void) {
  invoke("download_asr_model", { id: DEFAULT_MODEL }).catch((e) => onErr(String(e)));
}

export function Setup({ onReady, onSkip }: { onReady: () => void; onSkip: () => void }) {
  const [pct, setPct] = useState(0);
  const [err, setErr] = useState<string | null>(null);
  // Latest onReady, so the effect can run exactly ONCE. Without this, a parent
  // re-render (e.g. the version resolving) changes onReady's identity, re-runs
  // the effect, and fires a second download that the in-flight guard rejects —
  // showing a spurious "already downloading" error over a working download.
  const onReadyRef = useRef(onReady);
  useEffect(() => {
    onReadyRef.current = onReady;
  }, [onReady]);

  useEffect(() => {
    const unP = listen<{ id: string; downloaded: number; total: number }>("asr_model:progress", (e) => {
      if (e.payload.id === DEFAULT_MODEL && e.payload.total) {
        setPct(Math.round((e.payload.downloaded / e.payload.total) * 100));
      }
    });
    const unD = listen<string>("asr_model:done", (e) => {
      if (e.payload === DEFAULT_MODEL) onReadyRef.current();
    });
    if (!autoStarted) {
      autoStarted = true;
      beginDownload(setErr); // once per session; state updates arrive via events
    }
    return () => {
      unP.then((f) => f()).catch(() => {});
      unD.then((f) => f()).catch(() => {});
    };
  }, []);

  const retry = () => {
    setErr(null);
    setPct(0);
    beginDownload(setErr);
  };

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "var(--bg)",
        zIndex: 40,
        display: "grid",
        placeItems: "center",
        padding: 24,
      }}
    >
      <div style={{ maxWidth: 440, width: "100%", display: "grid", gap: 20, justifyItems: "center", textAlign: "center" }}>
        <Mark size={40} />
        <h1 style={{ fontSize: 22, margin: 0, letterSpacing: "-0.01em" }}>Getting Anchor ready</h1>
        {!err ? (
          <>
            <p style={{ color: "var(--text-muted)", fontSize: 14, margin: 0, lineHeight: 1.55 }}>
              Downloading the speech model so Anchor can hear your calls. This happens once — then it
              works offline.
            </p>
            <div style={{ width: "100%", height: 8, borderRadius: 5, background: "var(--bg-elevated)", overflow: "hidden" }}>
              <div style={{ width: `${pct}%`, height: "100%", background: "var(--accent)", transition: "width 200ms" }} />
            </div>
            <div style={{ color: "var(--text-dim)", fontSize: 13 }}>{pct}%</div>
          </>
        ) : (
          <>
            <p style={{ color: "var(--red)", fontSize: 13.5, margin: 0, lineHeight: 1.55 }}>
              Couldn&apos;t download the model. Check your connection and try again.
            </p>
            <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
              <button
                className="press"
                onClick={retry}
                style={{
                  background: "var(--accent)",
                  color: "var(--bg)",
                  border: "none",
                  borderRadius: 8,
                  padding: "9px 18px",
                  fontSize: 14,
                  fontWeight: 600,
                  cursor: "pointer",
                }}
              >
                Try again
              </button>
              <button
                onClick={onSkip}
                style={{ background: "none", border: "none", color: "var(--text-dim)", fontSize: 13, cursor: "pointer" }}
              >
                Skip for now
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
