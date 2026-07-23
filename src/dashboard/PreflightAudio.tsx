//! Pre-flight audio check for the session flow (09_PLAN Phase 6): make sure
//! Anchor hears BOTH sides before the call. Reuses the Phase-4 capture
//! (start/stop_audio) + the audio:health dead-channel signal + asr:partial
//! activity. The dead-channel warning is the load-bearing one — with some
//! Bluetooth headsets Windows silently moves the output and "them" goes quiet.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { panel, btnGhost } from "./ui";

interface Health {
  them_silent: boolean;
  me_silent: boolean;
}

function ChannelRow({ label, on, silent, text }: { label: string; on: boolean; silent: boolean; text: string }) {
  const live = on && !silent;
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
      <span
        className={live ? "livePulse" : undefined}
        style={{
          width: 9,
          height: 9,
          borderRadius: "50%",
          background: live ? "var(--green)" : "var(--text-dim)",
          flexShrink: 0,
        }}
      />
      <span style={{ fontSize: 13.5, minWidth: 150 }}>{label}</span>
      <span style={{ fontSize: 12.5, color: "var(--text-dim)", flex: 1, fontFamily: "var(--font-mono)" }}>
        {!on ? "—" : silent ? "silent" : text || "hearing you…"}
      </span>
    </div>
  );
}

export function PreflightAudio() {
  const [on, setOn] = useState(false);
  const [health, setHealth] = useState<Health | null>(null);
  const [them, setThem] = useState("");
  const [me, setMe] = useState("");
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    const unH = listen<Health>("audio:health", (e) => setHealth(e.payload));
    const unP = listen<{ speaker: string; text: string }>("asr:partial", (e) => {
      (e.payload.speaker === "them" ? setThem : setMe)(e.payload.text);
    });
    return () => {
      unH.then((f) => f()).catch(() => {});
      unP.then((f) => f()).catch(() => {});
      invoke("stop_audio").catch(() => {}); // never leave capture running when we unmount
    };
  }, []);

  const toggle = async () => {
    setErr(null);
    try {
      if (on) {
        await invoke("stop_audio");
        setOn(false);
        setHealth(null);
        setThem("");
        setMe("");
      } else {
        await invoke("start_audio");
        setOn(true);
      }
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <section style={{ ...panel, display: "grid", gap: 12 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
        <button className="press" onClick={toggle} style={btnGhost}>
          {on ? "Stop check" : "Check audio"}
        </button>
        <span style={{ fontSize: 12.5, color: "var(--text-muted)" }}>
          Make sure Anchor hears both sides before you take the call.
        </span>
      </div>
      {err && <div style={{ color: "var(--red)", fontSize: 12.5 }}>{err}</div>}
      {on && (
        <div style={{ display: "grid", gap: 8 }}>
          <ChannelRow label="You (microphone)" on={on} silent={health?.me_silent ?? true} text={me} />
          <ChannelRow label="The other side (system audio)" on={on} silent={health?.them_silent ?? true} text={them} />
          {health?.them_silent && (
            <div style={{ color: "var(--assembled)", fontSize: 12.5, lineHeight: 1.5 }}>
              ⚠ We are not hearing the other side. Check your output device — with some Bluetooth
              headsets Windows moves the audio when a call starts, and then the other side is not
              transcribed.
            </div>
          )}
        </div>
      )}
    </section>
  );
}
