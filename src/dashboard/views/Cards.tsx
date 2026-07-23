//! Cards — your anchor library. One obvious thing to do: paste, click, done
//! (owner: "like ChatGPT — one field, you throw text in and it just works").
//! Anchor auto-detects raw study material vs ready cards. Everything else is
//! tucked behind "More options".

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { panel, btn, SectionTitle } from "../ui";

interface CardRow {
  id: string;
  title: string;
  tags: string | null;
  language: string;
  source: string;
  bullets: string[];
}
interface ImportReport {
  imported: number;
  warnings: string[];
  rejected: string[];
}
interface GenerateReport {
  markdown: string;
  chunks: number;
  cards: number;
  warnings: string[];
}

const looksLikeReadyCards = (t: string) => /^\s*##\s+\S/m.test(t);

// Native <option> popups ignore most inherited styling — set colours
// explicitly so the dropdown is readable in dark mode.
const optStyle: React.CSSProperties = { background: "#161b22", color: "#e8edf2" };

export function Cards() {
  const [cards, setCards] = useState<CardRow[]>([]);
  const [text, setText] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [progress, setProgress] = useState<string | null>(null);
  const [style, setStyle] = useState("default");
  const [review, setReview] = useState(false);
  const [expanded, setExpanded] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<CardRow[]>("list_cards").then(setCards).catch((e) => setErr(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    invoke<{ bullet_style: string }>("get_llm_config")
      .then((c) => setStyle(c.bullet_style))
      .catch(() => {});
    invoke<number>("retighten_corpus")
      .then((n) => n > 0 && refresh())
      .catch(() => {});
    const unGen = listen<{ done: number; total: number }>("ingest:progress", (e) =>
      setProgress(`working… ${e.payload.done}/${e.payload.total}`),
    );
    const unAd = listen<{ done: number; total: number }>("adapt:progress", (e) =>
      setProgress(`restyling… ${e.payload.done}/${e.payload.total}`),
    );
    return () => {
      unGen.then((f) => f()).catch(() => {});
      unAd.then((f) => f()).catch(() => {});
    };
  }, [refresh]);

  // The one primary action: figure out what the text is and add it.
  const add = async () => {
    if (!text.trim()) return;
    setErr(null);
    setInfo(null);
    try {
      if (looksLikeReadyCards(text)) {
        setBusy("adding your cards…");
        const r = await invoke<ImportReport>("import_cards", { markdown: text });
        setInfo(`Added ${r.imported} card${r.imported === 1 ? "" : "s"}.`);
        setText("");
      } else {
        setBusy(review ? "drafting cards…" : "building your cards…");
        const r = await invoke<GenerateReport>("generate_cards", { text, auto: !review });
        if (review) {
          setText(r.markdown);
          setInfo(`Drafted ${r.cards} card${r.cards === 1 ? "" : "s"} — edit above, then Add.`);
        } else {
          setInfo(`Built and added ${r.cards} card${r.cards === 1 ? "" : "s"} from your material.`);
          setText("");
        }
      }
      refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  const importFolder = async () => {
    const dir = await open({ directory: true, title: "Pick a folder of card files" });
    if (typeof dir !== "string") return;
    setBusy("importing folder…");
    setErr(null);
    try {
      const r = await invoke<ImportReport>("import_folder", { path: dir });
      setInfo(`Imported ${r.imported} cards from the folder.`);
      refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
    }
  };

  const changeStyle = async (s: string) => {
    setStyle(s);
    try {
      await invoke("set_llm_config", { bulletStyle: s });
      await invoke("restyle_card");
      refresh();
      const n = await invoke<number>("adapt_corpus");
      if (n > 0) {
        await invoke("restyle_card");
        refresh();
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      setProgress(null);
    }
  };

  const remove = async (id: string) => {
    await invoke("delete_card", { cardId: id }).catch((e) => setErr(String(e)));
    refresh();
  };

  return (
    <div style={{ maxWidth: 820, margin: "0 auto", display: "grid", gap: 24 }}>
      <div>
        <h1 style={{ fontSize: 26, margin: "0 0 4px", letterSpacing: "-0.01em" }}>Cards</h1>
        <p style={{ color: "var(--text-muted)", fontSize: 14.5, margin: 0 }}>
          Paste what you studied — Anchor turns it into short anchor cards. Six keywords, never
          sentences.
        </p>
      </div>

      {/* The one primary input */}
      <section
        style={{
          ...panel,
          padding: 6,
          borderColor: "var(--border)",
          display: "grid",
          gap: 0,
        }}
      >
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={7}
          placeholder="Paste your notes, a document, a knowledge-base dump — or ready-made cards. Anchor figures out the rest."
          style={{
            width: "100%",
            boxSizing: "border-box",
            background: "transparent",
            border: "none",
            outline: "none",
            resize: "vertical",
            color: "var(--text)",
            padding: 14,
            fontSize: 15,
            lineHeight: 1.5,
          }}
        />
        {/* Primary action + a calm, always-there options row (no toggle). */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            padding: "10px 12px",
            borderTop: "1px solid var(--border-soft)",
            flexWrap: "wrap",
          }}
        >
          <button
            className="press"
            onClick={add}
            disabled={!text.trim() || busy !== null}
            style={{ ...btn, opacity: !text.trim() || busy ? 0.55 : 1 }}
          >
            {review ? "Draft cards" : "Add to library"}
          </button>

          {/* Options — quiet pills, aligned right, small and unobtrusive */}
          <div
            style={{
              marginLeft: "auto",
              display: "flex",
              alignItems: "center",
              gap: 6,
              fontSize: 12.5,
              color: "var(--text-muted)",
            }}
          >
            <button
              onClick={() => setReview((r) => !r)}
              title="Look over the drafts before they go into your library"
              className="press"
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                padding: "6px 10px",
                borderRadius: 8,
                cursor: "pointer",
                background: review ? "var(--accent-bg)" : "transparent",
                border: `1px solid ${review ? "var(--accent)" : "var(--border)"}`,
                color: review ? "var(--accent)" : "var(--text-muted)",
              }}
            >
              {review ? "✓ Review first" : "Review first"}
            </button>

            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                padding: "4px 6px 4px 10px",
                borderRadius: 8,
                border: "1px solid var(--border)",
              }}
            >
              <span>Length</span>
              <select
                value={style}
                onChange={(e) => changeStyle(e.target.value)}
                style={{
                  background: "var(--bg-elevated)",
                  color: "var(--text)",
                  colorScheme: "dark",
                  border: "none",
                  outline: "none",
                  cursor: "pointer",
                  fontSize: 12.5,
                }}
              >
                <option style={optStyle} value="default">
                  Recommended
                </option>
                <option style={optStyle} value="short">
                  Short
                </option>
                <option style={optStyle} value="long">
                  Longer
                </option>
              </select>
            </div>

            <button
              onClick={importFolder}
              className="press"
              title="Import a folder of .md card files"
              style={{
                padding: "6px 10px",
                borderRadius: 8,
                cursor: "pointer",
                background: "transparent",
                border: "1px solid var(--border)",
                color: "var(--text-muted)",
              }}
            >
              Import folder
            </button>
          </div>
        </div>

        {(busy || progress) && (
          <div style={{ padding: "0 14px 10px", fontSize: 12.5, color: "var(--accent)" }}>
            {busy || progress}
          </div>
        )}
      </section>

      {err && (
        <div style={{ ...panel, borderColor: "var(--red)", color: "var(--red)", fontSize: 13 }}>{err}</div>
      )}
      {info && !busy && <div style={{ fontSize: 13, color: "var(--accent)" }}>{info}</div>}

      {/* Library */}
      <section>
        <SectionTitle>{cards.length ? `Library — ${cards.length} cards` : "Library"}</SectionTitle>
        {cards.length === 0 ? (
          <p style={{ color: "var(--text-muted)", fontSize: 14, margin: 0, lineHeight: 1.55 }}>
            No cards yet. Anchor does not know anything until you teach it — that is not a
            limitation, that is the product.
          </p>
        ) : (
          <div style={{ display: "grid", gap: 6 }}>
            {cards.map((c) => (
              <div key={c.id} className="lift" style={{ ...panel, padding: "12px 14px" }}>
                <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
                  <button
                    onClick={() => setExpanded(expanded === c.id ? null : c.id)}
                    style={{ background: "none", border: "none", color: "var(--text)", cursor: "pointer", fontSize: 14.5, textAlign: "left", padding: 0 }}
                  >
                    {c.title}
                  </button>
                  <span style={{ color: "var(--text-dim)", fontSize: 12, flex: 1 }}>
                    {c.bullets.length} anchors · {c.language}
                    {c.tags ? ` · ${c.tags}` : ""}
                  </span>
                  <button
                    onClick={() => remove(c.id)}
                    title="Delete this card"
                    style={{ background: "none", border: "none", color: "var(--text-dim)", cursor: "pointer" }}
                  >
                    ✕
                  </button>
                </div>
                {expanded === c.id && (
                  <ul style={{ margin: "8px 0 2px", paddingLeft: 18, color: "var(--text-soft)", fontSize: 13 }}>
                    {c.bullets.map((b, i) => (
                      <li key={i}>{b}</li>
                    ))}
                  </ul>
                )}
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
