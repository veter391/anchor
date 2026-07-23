//! One session's detail: the cards for THIS call, the Go-live control, and —
//! once the call ends — the coverage report (which anchors you hit and which
//! you missed: "what you failed to say", the feature that makes the philosophy
//! true in code). Pull cards from your library or paste fresh material.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { panel, btn, btnGhost, SectionTitle } from "../ui";
import { PreflightAudio } from "../PreflightAudio";

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
interface BulletCov {
  text: string;
  covered: boolean;
}
interface CardCov {
  card_id: string;
  title: string;
  came_up: boolean;
  bullets: BulletCov[];
  covered: number;
  total: number;
}
interface Report {
  cards: CardCov[];
  covered: number;
  total: number;
  verdict: string;
  untouched_cards: number;
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
  const [researchUrl, setResearchUrl] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [live, setLive] = useState(false);
  const [status, setStatus] = useState(session.status);
  const [report, setReport] = useState<Report | null>(null);
  const [shareHidden, setShareHidden] = useState(false);

  const closed = status === "closed_green" || status === "closed_red";

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
    invoke<boolean>("get_capture_excluded").then(setShareHidden).catch(() => {});
    if (session.status === "closed_green" || session.status === "closed_red") {
      invoke<Report>("session_report", { sessionId: session.id })
        .then(setReport)
        .catch((e) => setErr(String(e)));
    }
  }, [refresh, session.id, session.status]);

  const goLive = async () => {
    setErr(null);
    try {
      await invoke("set_active_session", { sessionId: session.id });
      setLive(true);
      setStatus("live");
    } catch (e) {
      setErr(String(e));
    }
  };
  const cancelLive = async () => {
    setErr(null);
    try {
      await invoke("clear_active_session");
      setLive(false);
      setStatus("planned");
    } catch (e) {
      setErr(String(e));
    }
  };
  const endCall = async () => {
    setErr(null);
    try {
      const r = await invoke<Report>("close_session", { sessionId: session.id });
      setReport(r);
      setLive(false);
      setStatus(r.verdict === "green" ? "closed_green" : "closed_red");
    } catch (e) {
      setErr(String(e));
    }
  };
  const takeAgain = async () => {
    setErr(null);
    try {
      await invoke("reopen_session", { sessionId: session.id });
      setReport(null);
      setStatus("planned");
    } catch (e) {
      setErr(String(e));
    }
  };
  const toggleShare = async () => {
    const next = !shareHidden;
    setShareHidden(next);
    await invoke("set_capture_excluded", { on: next }).catch((e) => setErr(String(e)));
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

  const research = async () => {
    if (!researchUrl.trim()) return;
    setBusy("researching the page…");
    setErr(null);
    setInfo(null);
    try {
      const r = await invoke<{ title: string; bullets: number }>("preflight_research", {
        url: researchUrl,
        sessionId: session.id,
      });
      setInfo(`Added a context card: ${r.title} (${r.bullets} anchors).`);
      setResearchUrl("");
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
  const promote = async (id: string) => {
    setErr(null);
    setInfo(null);
    try {
      await invoke<number>("promote_cards_to_library", { cardIds: [id] });
      setInfo("Copied to your library for reuse.");
      invoke<CardRow[]>("list_cards").then(setLibrary).catch(() => {});
    } catch (e) {
      setErr(String(e));
    }
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
              background: STATUS_DOT[status] ?? "var(--text-dim)",
            }}
          />
          <h1 style={{ fontSize: 25, margin: 0, letterSpacing: "-0.01em" }}>{session.title}</h1>
          <span style={{ color: "var(--text-muted)", fontSize: 13, textTransform: "capitalize" }}>
            {session.kind}
          </span>
        </div>
        {!closed && (
          <>
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
                  <button className="press" onClick={endCall} style={btn}>
                    End call
                  </button>
                  <button
                    className="press"
                    onClick={toggleShare}
                    style={btnGhost}
                    title={
                      shareHidden
                        ? "Reveal the card in your screen share"
                        : "Keep the card out of your screen share"
                    }
                  >
                    {shareHidden ? "Show notes" : "Hide from share"}
                  </button>
                  <button className="press" onClick={cancelLive} style={btnGhost}>
                    Cancel
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
          </>
        )}
      </div>

      {err && (
        <div style={{ ...panel, borderColor: "var(--red)", color: "var(--red)", fontSize: 13 }}>{err}</div>
      )}

      {closed && report ? (
        <ReportView report={report} onAgain={takeAgain} sessionId={session.id} />
      ) : (
        <>
          {(info || busy) && (
            <div style={{ fontSize: 13, color: busy ? "var(--accent)" : "var(--green)" }}>
              {busy || info}
            </div>
          )}

          <PreflightAudio />

          {/* Pre-flight research → context card */}
          <section style={{ ...panel, display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
            <span style={{ fontSize: 13.5, color: "var(--text-muted)" }}>Pre-flight research</span>
            <input
              value={researchUrl}
              onChange={(e) => setResearchUrl(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && researchUrl.trim() && busy === null && research()}
              placeholder="Paste the company or job-posting URL — Anchor builds a context card"
              style={{
                flex: 1,
                minWidth: 240,
                background: "var(--bg-elevated)",
                border: "1px solid var(--border)",
                borderRadius: 8,
                color: "var(--text)",
                padding: "9px 12px",
                fontSize: 13.5,
              }}
            />
            <button
              className="press"
              onClick={research}
              disabled={!researchUrl.trim() || busy !== null}
              style={{ ...btnGhost, opacity: !researchUrl.trim() || busy ? 0.55 : 1 }}
            >
              Research
            </button>
          </section>


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
                    <button
                      className="press"
                      onClick={() => promote(c.id)}
                      title="Copy this card to your library so you can reuse it in other sessions"
                      style={{ ...btnGhost, padding: "4px 10px", fontSize: 12 }}
                    >
                      ↑ Library
                    </button>
                    <button onClick={() => remove(c.id)} title="Remove from this session" style={{ background: "none", border: "none", color: "var(--text-dim)", cursor: "pointer" }}>
                      ✕
                    </button>
                  </div>
                ))}
              </div>
            )}
          </section>
        </>
      )}
    </div>
  );
}

interface TranscriptLine {
  speaker: string;
  ts_ms: number;
  text: string;
}

function ReportView({
  report,
  onAgain,
  sessionId,
}: {
  report: Report;
  onAgain: () => void;
  sessionId: string;
}) {
  const green = report.verdict === "green";
  const cameUp = report.cards.filter((c) => c.came_up);
  const untouched = report.cards.filter((c) => !c.came_up);
  const accent = green ? "var(--green)" : "var(--red)";
  const [transcript, setTranscript] = useState<TranscriptLine[]>([]);
  const [showTranscript, setShowTranscript] = useState(false);
  useEffect(() => {
    invoke<TranscriptLine[]>("session_transcript", { sessionId })
      .then(setTranscript)
      .catch(() => {});
  }, [sessionId]);
  return (
    <div style={{ display: "grid", gap: 18 }}>
      <div className="rise-in" style={{ ...panel, borderColor: accent, display: "flex", alignItems: "center", gap: 14 }}>
        <span style={{ width: 12, height: 12, borderRadius: "50%", background: accent, flexShrink: 0 }} />
        <div>
          <div style={{ fontSize: 16, fontWeight: 600 }}>
            {report.total === 0
              ? "No prepared topics came up."
              : green
                ? "You covered your ground."
                : "A few anchors slipped."}
          </div>
          <div style={{ color: "var(--text-muted)", fontSize: 13.5, marginTop: 2 }}>
            {report.total > 0
              ? `You hit ${report.covered} of ${report.total} anchors across the ${cameUp.length} card${cameUp.length === 1 ? "" : "s"} that came up.`
              : "None of this session's cards were triggered during the call."}
          </div>
        </div>
        <button className="press" onClick={onAgain} style={{ ...btnGhost, marginLeft: "auto" }}>
          Take again
        </button>
      </div>

      {cameUp.map((c) => (
        <section key={c.card_id}>
          <SectionTitle>
            {c.title} — {c.covered}/{c.total}
          </SectionTitle>
          <div style={{ display: "grid", gap: 5 }}>
            {c.bullets.map((b, i) => (
              <div
                key={i}
                style={{
                  display: "flex",
                  alignItems: "baseline",
                  gap: 8,
                  fontSize: 14,
                  color: b.covered ? "var(--text-dim)" : "var(--text)",
                }}
              >
                <span style={{ color: b.covered ? "var(--green)" : "var(--red)", width: 14, flexShrink: 0 }}>
                  {b.covered ? "✓" : "○"}
                </span>
                <span style={{ flex: 1 }}>{b.text}</span>
                {!b.covered && (
                  <span style={{ color: "var(--red)", fontSize: 11.5 }}>missed</span>
                )}
              </div>
            ))}
          </div>
        </section>
      ))}

      {untouched.length > 0 && (
        <p style={{ color: "var(--text-dim)", fontSize: 13, margin: 0, lineHeight: 1.5 }}>
          {untouched.length} card{untouched.length === 1 ? "" : "s"} didn't come up:{" "}
          {untouched.map((c) => c.title).join(", ")}.
        </p>
      )}

      {transcript.length > 0 && (
        <div>
          <button
            className="link"
            onClick={() => setShowTranscript((s) => !s)}
            style={{ background: "none", border: "none", fontSize: 13, padding: 0 }}
          >
            {showTranscript ? "Hide transcript" : `Transcript — ${transcript.length} lines`}
          </button>
          {showTranscript && (
            <div style={{ ...panel, marginTop: 8, display: "grid", gap: 5, maxHeight: 360, overflowY: "auto" }}>
              {transcript.map((t, i) => (
                <div key={i} style={{ fontSize: 13, display: "flex", gap: 10, alignItems: "baseline" }}>
                  <span
                    style={{
                      color: t.speaker === "them" ? "var(--accent)" : "var(--text-muted)",
                      minWidth: 42,
                      fontSize: 11,
                      textTransform: "uppercase",
                      letterSpacing: "0.03em",
                    }}
                  >
                    {t.speaker}
                  </span>
                  <span style={{ flex: 1, color: "var(--text-soft)", lineHeight: 1.5 }}>{t.text}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
