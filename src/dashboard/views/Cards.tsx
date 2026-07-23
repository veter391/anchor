//! Cards — the card library and the ways to fill it: generate from raw
//! material, import ready markdown, browse/delete. The friendly everyday
//! surface (no debug). Self-contained: talks to the backend directly.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { panel, btn, btnGhost, SectionTitle } from "../ui";

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
  imported: ImportReport | null;
}

const field: React.CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  background: "var(--bg-elevated)",
  border: "1px solid var(--border)",
  borderRadius: 8,
  color: "var(--text)",
  padding: 12,
  fontSize: 14,
};

export function Cards() {
  const [cards, setCards] = useState<CardRow[]>([]);
  const [raw, setRaw] = useState("");
  const [md, setMd] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [progress, setProgress] = useState<string | null>(null);
  const [style, setStyle] = useState("default");
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
      setProgress(`part ${e.payload.done}/${e.payload.total}`),
    );
    const unAd = listen<{ done: number; total: number }>("adapt:progress", (e) =>
      setProgress(`adapting cards ${e.payload.done}/${e.payload.total}`),
    );
    return () => {
      unGen.then((f) => f()).catch(() => {});
      unAd.then((f) => f()).catch(() => {});
    };
  }, [refresh]);

  const doGenerate = async (auto: boolean) => {
    setBusy(auto ? "generating + importing…" : "generating drafts…");
    setErr(null);
    setInfo(null);
    try {
      const r = await invoke<GenerateReport>("generate_cards", { text: raw, auto });
      if (auto) {
        setInfo(`generated ${r.cards} card(s) from ${r.chunks} part(s) — imported`);
        setRaw("");
        refresh();
      } else {
        setMd(r.markdown);
        setInfo(`generated ${r.cards} draft card(s) — review below, then Import`);
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
      setProgress(null);
    }
  };

  const doImport = async () => {
    setBusy("importing…");
    setErr(null);
    try {
      const r = await invoke<ImportReport>("import_cards", { markdown: md });
      setInfo(`imported ${r.imported} card(s)` + (r.warnings.length ? ` · ${r.warnings.length} warnings` : ""));
      setMd("");
      refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
    }
  };

  const doImportFolder = async () => {
    const dir = await open({ directory: true, title: "Pick your cards folder" });
    if (typeof dir !== "string") return;
    setBusy("importing folder…");
    setErr(null);
    try {
      const r = await invoke<ImportReport>("import_folder", { path: dir });
      setInfo(`imported ${r.imported} card(s) from folder`);
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

  const doDelete = async (id: string) => {
    try {
      await invoke("delete_card", { cardId: id });
      refresh();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div style={{ maxWidth: 920, margin: "0 auto", display: "grid", gap: 22 }}>
      <div>
        <h1 style={{ fontSize: 24, margin: "0 0 4px" }}>Cards</h1>
        <p style={{ color: "var(--text-muted)", fontSize: 14, margin: 0 }}>
          Your anchor library. Six keywords per card, never sentences — a script will fail you.
        </p>
      </div>

      {err && (
        <div style={{ ...panel, borderColor: "var(--red)", color: "var(--red)", fontSize: 13 }}>
          {err}
        </div>
      )}
      {(info || busy || progress) && (
        <div style={{ fontSize: 13, color: busy ? "var(--accent)" : "var(--assembled)" }}>
          {busy || progress || info}
        </div>
      )}

      {/* Generate from raw material */}
      <section style={panel}>
        <SectionTitle hint="Drop your study notes as-is — no formatting. The engine drafts anchor cards; review them or import automatically.">
          Generate from material
        </SectionTitle>
        <textarea
          value={raw}
          onChange={(e) => setRaw(e.target.value)}
          rows={6}
          placeholder="Paste the material you studied — meeting notes, a doc, a knowledge-base dump…"
          style={field}
        />
        <div style={{ marginTop: 10, display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          <button onClick={() => doGenerate(false)} disabled={!raw.trim() || busy !== null} style={btn}>
            Generate drafts
          </button>
          <button onClick={() => doGenerate(true)} disabled={!raw.trim() || busy !== null} style={btnGhost}>
            Generate &amp; import
          </button>
          <label style={{ color: "var(--text-muted)", fontSize: 12, marginLeft: "auto" }}>
            Bullet length{" "}
            <select
              value={style}
              onChange={(e) => changeStyle(e.target.value)}
              style={{
                background: "var(--bg-elevated)",
                color: "var(--text)",
                border: "1px solid var(--border)",
                borderRadius: 6,
                padding: "5px 8px",
              }}
            >
              <option value="default">Recommended</option>
              <option value="short">Short (1-2 words)</option>
              <option value="long">Longer</option>
            </select>
          </label>
        </div>
      </section>

      {/* Import ready markdown */}
      <section style={panel}>
        <SectionTitle hint="Already have cards in markdown? Paste or import a folder of .md files.">
          Import markdown
        </SectionTitle>
        <textarea
          value={md}
          onChange={(e) => setMd(e.target.value)}
          rows={5}
          placeholder={"## Question phrased like a person would ask it\n\n- Anchor one\n- Anchor two"}
          style={{ ...field, fontFamily: "var(--font-mono)", fontSize: 13 }}
        />
        <div style={{ marginTop: 10, display: "flex", gap: 8 }}>
          <button onClick={doImport} disabled={!md.trim() || busy !== null} style={btn}>
            Import
          </button>
          <button onClick={doImportFolder} disabled={busy !== null} style={btnGhost}>
            Import folder…
          </button>
        </div>
      </section>

      {/* Library */}
      <section style={panel}>
        <SectionTitle>{cards.length ? `Library — ${cards.length} cards` : "Library"}</SectionTitle>
        {cards.length === 0 ? (
          <div style={{ color: "var(--text-dim)", fontSize: 14 }}>
            No cards yet. Anchor does not know anything until you teach it. That is not a
            limitation — that is the product.
          </div>
        ) : (
          <div style={{ display: "grid", gap: 6 }}>
            {cards.map((c) => (
              <div key={c.id} style={{ borderLeft: "3px solid var(--border)", paddingLeft: 12 }}>
                <div style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
                  <button
                    onClick={() => setExpanded(expanded === c.id ? null : c.id)}
                    style={{ background: "none", border: "none", color: "var(--text)", cursor: "pointer", fontSize: 14, textAlign: "left", padding: 0 }}
                  >
                    {c.title}
                  </button>
                  <span style={{ color: "var(--text-dim)", fontSize: 12, flex: 1 }}>
                    {c.bullets.length} anchors · {c.language}
                    {c.tags ? ` · ${c.tags}` : ""}
                  </span>
                  <button
                    onClick={() => doDelete(c.id)}
                    title="Delete this card"
                    style={{ background: "none", border: "none", color: "var(--text-dim)", cursor: "pointer" }}
                  >
                    ✕
                  </button>
                </div>
                {expanded === c.id && (
                  <ul style={{ margin: "6px 0 10px", paddingLeft: 18, color: "var(--text-soft)", fontSize: 13 }}>
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
