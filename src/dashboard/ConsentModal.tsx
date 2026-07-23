//! First-run consent + transparency screen (08_LEGAL §5). Shown once, before
//! the first call: what Anchor captures (both channels, locally), what it stores
//! (text only), the three egress points, the recording-law responsibility, and
//! the model-download terms. Plain language, no scroll-wall.

import { panel, btn } from "./ui";

export function ConsentModal({ onAccept }: { onAccept: () => void }) {
  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 100,
        background: "color-mix(in srgb, var(--bg) 80%, transparent)",
        backdropFilter: "blur(5px)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: 24,
      }}
    >
      <div className="rise-in" style={{ ...panel, maxWidth: 640, padding: 28, display: "grid", gap: 16 }}>
        <div>
          <h1 style={{ fontSize: 22, margin: "0 0 4px", letterSpacing: "-0.01em" }}>Before your first call</h1>
          <p style={{ color: "var(--text-muted)", fontSize: 14, margin: 0 }}>
            Anchor is local-first and honest about what it does. Thirty seconds, just once.
          </p>
        </div>
        <ul style={{ margin: 0, padding: 0, listStyle: "none", display: "grid", gap: 11, fontSize: 14, lineHeight: 1.55 }}>
          <li>
            🎙️ <b>It listens to both sides of the call, on your machine.</b> Your microphone is
            you; the system audio is the other side. Nothing is streamed to a server.
          </li>
          <li>
            📝 <b>It stores text only.</b> Audio is processed in memory and never written to disk.
          </li>
          <li>
            🌐 <b>Almost nothing leaves your machine.</b> The only network calls are the optional
            bring-your-own-key assistant, a web page you explicitly ask it to fetch, and one-time
            model downloads.
          </li>
          <li>
            ⚖️ <b>Recording law is your responsibility.</b> Transcribing another person can require
            their consent depending on where you are. Best practice: say at the start of the call
            that you are taking notes.
          </li>
          <li>
            📦 <b>Models download on first use</b> from Hugging Face, under their own licences
            (including Google&apos;s Gemma terms for the embedding model).
          </li>
        </ul>
        <div style={{ display: "flex", gap: 12, alignItems: "center", flexWrap: "wrap" }}>
          <button className="press" onClick={onAccept} style={btn}>
            I understand — let&apos;s go
          </button>
          <span style={{ color: "var(--text-dim)", fontSize: 12.5 }}>
            You can revisit all of this any time under About.
          </span>
        </div>
      </div>
    </div>
  );
}
