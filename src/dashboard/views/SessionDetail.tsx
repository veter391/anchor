//! One session's detail: the cards that belong to THIS call. Pull from your
//! library or paste fresh material. (Live transcript + coverage report land in
//! the next increment.) Kept light, one obvious action.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { panel, btn, btnGhost, SectionTitle } from "../ui";

interface CardRow {
  id: string;
  title: string;
  tags: string | null;
  language: string;
  source: string;
  bullets: string[];
}
interface SessionRow {
  id: string;
  title: string;
  kind: string;
  status: string;
}

const STATUS_DOT: Record<string, string> = {
  planned: "var(--text-dim)",
  live: "var(--accent)",
  closed_green: "var(--green)",
  closed_red: "var(--red)",
};

export function SessionDetail({ session, onBack }: { session: SessionRow; onBack: () => void }) {
  const [cards, setCards] = useState<CardRow[]>([]);
  const [library, setLibrary] = useState<CardRow[]>([]);
  const [showLibrary, setShowLibrary] = useState(false);
  const [material, setMaterial] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [live, setLive] = useState(false);

  const refresh = useCallback(() => {
    invoke<CardRow[]>("list_session_cards", { sessionId: session.id })
      .then(setCards)
      .catch((e) => setErr(String(e)));
  }, [session.id]);

  useEffect(() => {
    refresh();
    invoke<CardRow[]>("list_cards").then(setLibrary).catch(() => {});
    invoke<string>("get_active_session")
      .then((id) => setLive(id === session.id))
      .catch(() => {});
  }, [refresh, session.id]);

  const goLive = async () => {
    setErr(null);
    try {
      await invoke("set_active_session", { sessionId: session.id });
      setLive(true);
    } catch (e) {
      setErr(String(e));
    }
  };
  const stopLive = async () => {
    setErr(null);
    try {
      await invoke("clear_active_session");
      setLive(false);
    } catch (e) {
      setErr(String(e));
    }
  };

  const addMaterial = async () => {
    if (!material.trim()) return;
    setBusy("building cards…");
    setErr(null);
    setInfo(null);
    try {
      const r = await invoke<{ cards: number }>("generate_cards", {
        text: material,
        auto: true,
        sessionId: session.id,
      });
      setInfo(`Added ${r.cards} card${r.cards === 1 ? "" : "s"} to this session.`);
      setMaterial("");
      refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
    }
  };

  const addFromLibrary = async (id: string) => {
    setErr(null);
    try {
      await invoke<number>("add_library_cards_to_session", {
        sessionId: session.id,
        cardIds: [id],
      });
      refresh();
    } catch (e) {
      setErr(String(e));
    }
  };

  const remove = async (id: string) => {
    await invoke("delete_card", { cardId: id }).catch((e) => setErr(String(e)));
    refresh();
  };

  return (
    <div style={{ maxWidth: 860, margin: "0 auto", display: "grid", gap: 20 }}>
      <div>
        <button className="link" onClick={onBack} style={{ background: "none", border: "none", fontSize: 13, padding: 0 }}>
          ← All sessions
        </button>
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginTop: 8 }}>
          <span
            style={{
              width: 10,
              height: 10,
              borderRadius: "50%",
              background: STATUS_DOT[session.status] ?? "var(--text-dim)",
            }}
          />
          <h1 style={{ fontSize: 25, margin: 0, letterSpacing: "-0.01em" }}>{session.title}</h1>
          <span style={{ color: "var(--text-muted)", fontSize: 13, textTransform: "capitalize" }}>
            {session.kind}
          </span>
        </div>
        <p style={{ color: "var(--text-muted)", fontSize: 14, margin: "6px 0 0" }}>
          The cards you will lean on in this call. Pull from your library, or paste fresh material.
        </p>

        <div style={{ marginTop: 14 }}>
          {live ? (
            <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
              <span style={{ display: "inline-flex", alignItems: "center", gap: 8, color: "var(--accent)", fontSize: 14, fontWeight: 600 }}>
                <span className="livePulse" style={{ width: 9, height: 9, borderRadius: "50%", background: "var(--accent)" }} />
                Live — your overlay is tracking this call
              </span>
              <button className="press" onClick={stopLive} style={btnGhost}>
                Stop
              </button>
            </div>
          ) : (
            <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
              <button
                className="press"
                onClick={goLive}
                disabled={cards.length === 0}
                style={{ ...btn, opacity: cards.length === 0 ? 0.55 : 1 }}
                title={cards.length === 0 ? "Add at least one card first" : undefined}
              >
                Go live
              </button>
              <span style={{ color: "var(--text-dim)", fontSize: 13 }}>
                {cards.length === 0
                  ? "Add at least one card first."
                  : "Anchor will match only this session's cards as the call goes."}
              </span>
            </div>
          )}
        </div>
      </div>

      {err && (
        <div style={{ ...panel, borderColor: "var(--red)", color: "var(--red)", fontSize: 13 }}>{err}</div>
      )}
      {(info || busy) && (
        <div style={{ fontSize: 13, color: busy ? "var(--accent)" : "var(--green)" }}>
          {busy || info}
        </div>
      )}

      {/* Add material into this session */}
      <section style={{ ...panel, padding: 6 }}>
        <textarea
          value={material}
          onChange={(e) => setMaterial(e.target.value)}
          rows={4}
          placeholder="Paste material for this call — Anchor turns it into anchor cards, scoped to this session."
          style={{
            width: "100%",
            boxSizing: "border-box",
            background: "transparent",
            border: "none",
            outline: "none",
            resize: "vertical",
            color: "var(--text)",
            padding: 12,
            fontSize: 14.5,
            lineHeight: 1.5,
          }}
        />
        <div style={{ display: "flex", gap: 10, alignItems: "center", padding: "8px 10px", borderTop: "1px solid var(--border-soft)" }}>
          <button className="press" onClick={addMaterial} disabled={!material.trim() || busy !== null} style={{ ...btn, opacity: !material.trim() || busy ? 0.55 : 1 }}>
            Add to this session
          </button>
          <button
            className="press"
            onClick={() => setShowLibrary((s) => !s)}
            style={{ ...btnGhost, marginLeft: "auto" }}
          >
            {showLibrary ? "Hide library" : "Add from library"}
          </button>
        </div>
      </section>

      {/* Library picker */}
      {showLibrary && (
        <section style={panel}>
          <SectionTitle>Your library — pick cards to add</SectionTitle>
          {library.length === 0 ? (
            <p style={{ color: "var(--text-muted)", fontSize: 13.5, margin: 0 }}>
              Your library is empty. Build cards under Cards first, or paste material above.
            </p>
          ) : (
            <div style={{ display: "grid", gap: 6 }}>
              {library.map((c) => (
                <div key={c.id} style={{ display: "flex", alignItems: "center", gap: 8, borderLeft: "3px solid var(--border)", paddingLeft: 12 }}>
                  <span style={{ fontSize: 14, flex: 1 }}>{c.title}</span>
                  <span style={{ color: "var(--text-dim)", fontSize: 12 }}>{c.bullets.length} anchors</span>
                  <button className="press" onClick={() => addFromLibrary(c.id)} style={{ ...btnGhost, padding: "5px 12px", fontSize: 12.5 }}>
                    Add
                  </button>
                </div>
              ))}
            </div>
          )}
        </section>
      )}

      {/* This session's cards */}
      <section>
        <SectionTitle>{cards.length ? `Cards in this session — ${cards.length}` : "Cards in this session"}</SectionTitle>
        {cards.length === 0 ? (
          <p style={{ color: "var(--text-muted)", fontSize: 14, margin: 0, lineHeight: 1.55 }}>
            No cards yet. Add from your library or paste material above — then this session is ready
            to take the call.
          </p>
        ) : (
          <div style={{ display: "grid", gap: 6 }}>
            {cards.map((c) => (
              <div key={c.id} className="lift" style={{ ...panel, padding: "12px 14px", display: "flex", alignItems: "baseline", gap: 8 }}>
                <span style={{ fontSize: 14.5 }}>{c.title}</span>
                <span style={{ color: "var(--text-dim)", fontSize: 12, flex: 1 }}>
                  {c.bullets.length} anchors · {c.language}
                </span>
                <button onClick={() => remove(c.id)} title="Remove from this session" style={{ background: "none", border: "none", color: "var(--text-dim)", cursor: "pointer" }}>
                  ✕
                </button>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
