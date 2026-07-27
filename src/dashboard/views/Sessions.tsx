//! Sessions — your calls as folders. Each is one conversation: its cards,
//! transcript and coverage report. Grouped by kind (owner: "different chats /
//! different projects", no single global soup).

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { panel, btn, btnGhost, SectionTitle } from "../ui";
import { SessionDetail } from "./SessionDetail";

interface SessionRow {
  id: string;
  title: string;
  kind: string;
  status: string;
  language: string;
  created_at: number;
  card_count: number;
}

const KINDS = [
  { key: "interview", label: "Interviews" },
  { key: "client", label: "Client calls" },
  { key: "team", label: "Team calls" },
  { key: "investor", label: "Investor calls" },
  { key: "other", label: "Other" },
];

// The expected speech language, used to steer the on-device transcription.
// "Auto-detect" lets the multilingual model decide — the safe default. The
// launch languages are the ones we tune and benchmark (04_MODELS).
const LANGS = [
  { key: "auto", label: "Auto-detect" },
  { key: "en", label: "English" },
  { key: "es", label: "Spanish" },
  { key: "ru", label: "Russian" },
  { key: "uk", label: "Ukrainian" },
];

const STATUS_DOT: Record<string, string> = {
  planned: "var(--text-dim)",
  live: "var(--accent)",
  closed_green: "var(--green)",
  closed_red: "var(--red)",
  archived: "var(--text-dim)",
};

export function Sessions() {
  const [sessions, setSessions] = useState<SessionRow[]>([]);
  const [creating, setCreating] = useState(false);
  const [title, setTitle] = useState("");
  const [kind, setKind] = useState("interview");
  const [language, setLanguage] = useState("auto");
  const [err, setErr] = useState<string | null>(null);
  const [open, setOpenId] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<SessionRow[]>("list_sessions").then(setSessions).catch((e) => setErr(String(e)));
  }, []);
  useEffect(() => refresh(), [refresh]);

  const create = async () => {
    try {
      await invoke<string>("create_session", { title, kind, language });
      setTitle("");
      setCreating(false);
      refresh();
    } catch (e) {
      setErr(String(e));
    }
  };
  const remove = async (id: string) => {
    await invoke("delete_session", { id }).catch((e) => setErr(String(e)));
    if (open === id) setOpenId(null);
    refresh();
  };
  const archive = async (id: string) => {
    await invoke("set_session_status", { id, status: "archived" }).catch((e) => setErr(String(e)));
    refresh();
  };

  const active = sessions.filter((s) => s.status !== "archived");
  const opened = open ? sessions.find((s) => s.id === open) : null;

  if (opened) {
    return (
      <SessionDetail
        session={opened}
        onBack={() => {
          setOpenId(null);
          refresh();
        }}
      />
    );
  }

  return (
    <div style={{ maxWidth: 920, margin: "0 auto", display: "grid", gap: 20 }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <div>
          <h1 style={{ fontSize: 24, margin: "0 0 4px" }}>Sessions</h1>
          <p style={{ color: "var(--text-muted)", fontSize: 14, margin: 0 }}>
            One session per call. Each keeps its own cards, transcript and coverage report.
          </p>
        </div>
        <button onClick={() => setCreating((c) => !c)} style={btn}>
          {creating ? "Cancel" : "New session"}
        </button>
      </div>

      {err && (
        <div style={{ ...panel, borderColor: "var(--red)", color: "var(--red)", fontSize: 13 }}>{err}</div>
      )}

      {creating && (
        <section style={panel}>
          <SectionTitle>New session</SectionTitle>
          <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
            <input
              autoFocus
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && title.trim() && create()}
              placeholder="e.g. Nexthink interview, Standup with the team…"
              style={{
                flex: 1,
                minWidth: 220,
                background: "var(--bg-elevated)",
                border: "1px solid var(--border)",
                borderRadius: 8,
                color: "var(--text)",
                padding: "10px 12px",
                fontSize: 14,
              }}
            />
            <select
              value={kind}
              onChange={(e) => setKind(e.target.value)}
              style={{
                background: "var(--bg-elevated)",
                color: "var(--text)",
                border: "1px solid var(--border)",
                borderRadius: 8,
                padding: "10px 12px",
              }}
            >
              {KINDS.map((k) => (
                <option key={k.key} value={k.key}>
                  {k.label.replace(/s$/, "")}
                </option>
              ))}
            </select>
            <select
              value={language}
              onChange={(e) => setLanguage(e.target.value)}
              title="The language you expect to be spoken on the call"
              style={{
                background: "var(--bg-elevated)",
                color: "var(--text)",
                border: "1px solid var(--border)",
                borderRadius: 8,
                padding: "10px 12px",
              }}
            >
              {LANGS.map((l) => (
                <option key={l.key} value={l.key}>
                  {l.label}
                </option>
              ))}
            </select>
            <button onClick={create} disabled={!title.trim()} style={btn}>
              Create
            </button>
          </div>
        </section>
      )}

      {active.length === 0 && !creating && (
        <div style={{ ...panel, color: "var(--text-muted)", fontSize: 14 }}>
          No sessions yet. Create one for your next call, drop your cards in, and take it.
        </div>
      )}

      {KINDS.map((k) => {
        const rows = active.filter((s) => s.kind === k.key);
        if (rows.length === 0) return null;
        return (
          <section key={k.key}>
            <SectionTitle>{k.label}</SectionTitle>
            <div style={{ display: "grid", gap: 8 }}>
              {rows.map((s) => (
                <div key={s.id} style={{ ...panel, padding: 14 }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                    <span
                      title={s.status}
                      style={{
                        width: 9,
                        height: 9,
                        borderRadius: "50%",
                        background: STATUS_DOT[s.status] ?? "var(--text-dim)",
                        flexShrink: 0,
                      }}
                    />
                    <button
                      className="link"
                      onClick={() => setOpenId(s.id)}
                      style={{ background: "none", border: "none", color: "var(--text)", fontSize: 15, cursor: "pointer", textAlign: "left", padding: 0 }}
                    >
                      {s.title}
                    </button>
                    <span style={{ color: "var(--text-dim)", fontSize: 12, flex: 1 }}>
                      {s.card_count} cards
                    </span>
                    <button className="press" onClick={() => setOpenId(s.id)} style={{ ...btnGhost, padding: "5px 10px", fontSize: 12 }}>
                      Open
                    </button>
                    <button onClick={() => archive(s.id)} style={{ ...btnGhost, padding: "5px 10px", fontSize: 12 }}>
                      Archive
                    </button>
                    <button
                      onClick={() => remove(s.id)}
                      style={{ background: "none", border: "none", color: "var(--text-dim)", cursor: "pointer" }}
                      title="Delete session"
                    >
                      ✕
                    </button>
                  </div>
                </div>
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
