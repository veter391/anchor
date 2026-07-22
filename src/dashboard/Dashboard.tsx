import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { LivePanel } from "./LivePanel";

interface BootInfo {
  db_path: string;
  schema_version: number;
  tables: string[];
  embedding_model: string;
  embedding_dims: number;
  embedder_loaded: boolean;
  cards: number;
}

interface ImportReport {
  imported: number;
  warnings: string[];
  rejected: string[];
}

interface CardRow {
  id: string;
  title: string;
  tags: string | null;
  language: string;
  source: string;
  bullets: string[];
}

interface Match {
  card_id: string;
  fused: number;
  vec_rank: number | null;
  vec_distance: number | null;
  bm25_rank: number | null;
}

interface QueryResult {
  matches: Match[];
  top_card: CardRow | null;
}

interface GenerateReport {
  markdown: string;
  chunks: number;
  cards: number;
  warnings: string[];
  imported: ImportReport | null;
}

const panel: React.CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border)",
  borderRadius: 8,
  padding: 14,
};

export function Dashboard() {
  const [boot, setBoot] = useState<BootInfo | null>(null);
  const [cards, setCards] = useState<CardRow[]>([]);
  const [md, setMd] = useState("");
  const [report, setReport] = useState<ImportReport | null>(null);
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<QueryResult | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [raw, setRaw] = useState("");
  const [genInfo, setGenInfo] = useState<string | null>(null);
  const [genProgress, setGenProgress] = useState<string | null>(null);
  const [style, setStyle] = useState("default");

  const pollFails = useRef(0);

  const refresh = useCallback(() => {
    invoke<BootInfo>("boot_info").then(setBoot).catch((e) => setErr(String(e)));
    invoke<CardRow[]>("list_cards").then(setCards).catch((e) => setErr(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(() => {
      // embedder_loaded flips asynchronously after the pre-warm finishes.
      // Repeated failures surface instead of silently showing stale state.
      invoke<BootInfo>("boot_info")
        .then((b) => {
          pollFails.current = 0;
          setBoot(b);
        })
        .catch((e) => {
          pollFails.current += 1;
          if (pollFails.current === 3) setErr(`status polling failing: ${e}`);
        });
    }, 2000);
    invoke<{ bullet_style: string }>("get_llm_config")
      .then((c) => setStyle(c.bullet_style))
      .catch(() => {});
    const unGen = listen<{ done: number; total: number }>("ingest:progress", (e) =>
      setGenProgress(`part ${e.payload.done}/${e.payload.total}`),
    );
    const unAdapt = listen<{ done: number; total: number }>("adapt:progress", (e) =>
      setGenProgress(`adapting cards ${e.payload.done}/${e.payload.total}`),
    );
    return () => {
      clearInterval(t);
      unGen.then((f) => f()).catch(() => {});
      unAdapt.then((f) => f()).catch(() => {});
    };
  }, [refresh]);

  const doImport = async () => {
    setBusy("importing…");
    setErr(null);
    try {
      setReport(await invoke<ImportReport>("import_cards", { markdown: md }));
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
      setReport(await invoke<ImportReport>("import_folder", { path: dir }));
      refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
    }
  };

  const doGenerate = async (auto: boolean) => {
    setBusy(auto ? "generating + importing…" : "generating drafts…");
    setErr(null);
    setGenInfo(null);
    try {
      const r = await invoke<GenerateReport>("generate_cards", { text: raw, auto });
      if (auto) {
        setGenInfo(
          `generated ${r.cards} card(s) from ${r.chunks} part(s) — imported into the corpus` +
            (r.warnings.length ? ` · ${r.warnings.length} part(s) skipped` : ""),
        );
        setRaw("");
        refresh();
      } else {
        setMd(r.markdown);
        setGenInfo(
          `generated ${r.cards} draft card(s) from ${r.chunks} part(s) — review below, then Import` +
            (r.warnings.length ? ` · ${r.warnings.length} part(s) skipped` : ""),
        );
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
      setGenProgress(null);
    }
  };

  const doWipe = async () => {
    setBusy("wiping…");
    setErr(null);
    try {
      await invoke("wipe_corpus");
      setReport(null);
      setResult(null);
      refresh();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
    }
  };

  const doDelete = async (cardId: string) => {
    setErr(null);
    try {
      await invoke("delete_card", { cardId });
      refresh();
    } catch (e) {
      setErr(String(e));
    }
  };

  const doQuery = async () => {
    setBusy("matching…");
    setErr(null);
    try {
      setResult(await invoke<QueryResult>("query_cards", { text: query }));
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(null);
    }
  };

  const titleOf = (id: string) => cards.find((c) => c.id === id)?.title ?? id;

  return (
    <div
      style={{
        minHeight: "100vh",
        background: "var(--bg-elevated)",
        color: "var(--text)",
        padding: 32,
        display: "grid",
        gap: 20,
        alignContent: "start",
      }}
    >
      <div>
        <h1 style={{ fontSize: 22, margin: 0 }}>
          ANCHOR{" "}
          <span style={{ color: "var(--text-muted)", fontSize: 14 }}>
            Prepared, not prompted.
          </span>
        </h1>
        {boot && (
          <div style={{ color: "var(--text-muted)", fontSize: 13, marginTop: 6 }}>
            {boot.cards} cards · {boot.embedding_model} ({boot.embedding_dims}d) ·{" "}
            {boot.embedder_loaded ? (
              <span style={{ color: "var(--green)" }}>embedder ready</span>
            ) : (
              <span style={{ color: "var(--assembled)" }}>
                embedder loading (first run downloads the model)…
              </span>
            )}
          </div>
        )}
        {err && <pre style={{ color: "var(--red)", fontSize: 12 }}>{err}</pre>}
        {busy && <div style={{ color: "var(--accent)", fontSize: 13 }}>{busy}</div>}
      </div>

      <section style={panel}>
        <h3 style={{ margin: "0 0 8px", fontSize: 13, color: "var(--text-muted)" }}>
          ASK — fake transcript (Phase 2)
        </h3>
        <div style={{ display: "flex", gap: 8 }}>
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && query.trim() && doQuery()}
            placeholder='e.g. "why are you leaving your company"'
            style={{
              flex: 1,
              background: "var(--bg-elevated)",
              border: "1px solid var(--border)",
              borderRadius: 6,
              color: "var(--text)",
              padding: "10px 12px",
              fontSize: 15,
            }}
          />
          <button onClick={doQuery} disabled={!query.trim() || busy !== null} style={btn}>
            Match
          </button>
        </div>
        {result && (
          <table style={{ marginTop: 10, fontSize: 13, borderSpacing: "0 4px" }}>
            <tbody>
              {result.matches.slice(0, 3).map((m, i) => (
                <tr key={m.card_id} style={{ color: i === 0 ? "var(--accent)" : "var(--text-muted)" }}>
                  <td style={{ paddingRight: 12 }}>#{i + 1}</td>
                  <td style={{ paddingRight: 12 }}>{titleOf(m.card_id)}</td>
                  <td style={{ paddingRight: 12, fontFamily: "var(--font-mono)" }}>
                    rrf {m.fused.toFixed(4)}
                  </td>
                  <td style={{ fontFamily: "var(--font-mono)" }}>
                    vec {m.vec_rank ?? "–"} · bm25 {m.bm25_rank ?? "–"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>

      <LivePanel />

      <section style={panel}>
        <h3 style={{ margin: "0 0 8px", fontSize: 13, color: "var(--text-muted)" }}>
          GENERATE CARDS — paste raw material, Anchor structures it
        </h3>
        <p style={{ color: "var(--text-muted)", fontSize: 12, marginTop: 0 }}>
          Drop your study notes as-is — no formatting needed. The engine turns
          them into draft anchor cards: review them in the import box below, or
          import automatically.
        </p>
        <textarea
          value={raw}
          onChange={(e) => setRaw(e.target.value)}
          rows={7}
          placeholder={
            "Paste the material you studied — meeting notes, a doc, a knowledge base dump…"
          }
          style={{
            width: "100%",
            boxSizing: "border-box",
            background: "var(--bg-elevated)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            color: "var(--text)",
            padding: 10,
            fontSize: 13,
          }}
        />
        <div style={{ marginTop: 8, display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          <button
            onClick={() => doGenerate(false)}
            disabled={!raw.trim() || busy !== null}
            style={btn}
          >
            Generate drafts
          </button>
          <button
            onClick={() => doGenerate(true)}
            disabled={!raw.trim() || busy !== null}
            style={btn}
          >
            Generate &amp; import
          </button>
          <label style={{ color: "var(--text-muted)", fontSize: 12 }}>
            Bullet length
            <select
              value={style}
              onChange={async (e) => {
                setStyle(e.target.value);
                try {
                  await invoke("set_llm_config", { bulletStyle: e.target.value });
                  // Whole corpus restyles in the moment: overlay re-emits its
                  // card; missing variants are backfilled in the background.
                  await invoke("restyle_card");
                  refresh();
                  const adapted = await invoke<number>("adapt_corpus");
                  if (adapted > 0) {
                    await invoke("restyle_card");
                    refresh();
                  }
                } catch (err) {
                  setErr(String(err));
                } finally {
                  setGenProgress(null);
                }
              }}
              style={{
                marginLeft: 6,
                background: "var(--bg-elevated)",
                color: "var(--text)",
                border: "1px solid var(--border)",
                borderRadius: 6,
                padding: "4px 8px",
              }}
            >
              <option value="default">Recommended</option>
              <option value="short">Short (1-2 words)</option>
              <option value="long">Longer</option>
            </select>
          </label>
          {genProgress && (
            <span style={{ color: "var(--assembled)", fontSize: 12 }}>{genProgress}</span>
          )}
        </div>
        {genInfo && (
          <div style={{ marginTop: 8, fontSize: 12, color: "var(--assembled)" }}>{genInfo}</div>
        )}
      </section>

      <section style={panel}>
        <h3 style={{ margin: "0 0 8px", fontSize: 13, color: "var(--text-muted)" }}>
          IMPORT CARDS — paste markdown
        </h3>
        <textarea
          value={md}
          onChange={(e) => setMd(e.target.value)}
          rows={8}
          placeholder={"## Question phrased like a person would ask it\ntags: hr\nlang: en\n\n- Anchor one\n- Anchor two"}
          style={{
            width: "100%",
            boxSizing: "border-box",
            background: "var(--bg-elevated)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            color: "var(--text)",
            padding: 10,
            fontFamily: "var(--font-mono)",
            fontSize: 13,
          }}
        />
        <div style={{ marginTop: 8, display: "flex", gap: 8 }}>
          <button onClick={doImport} disabled={!md.trim() || busy !== null} style={btn}>
            Import
          </button>
          <button onClick={doImportFolder} disabled={busy !== null} style={btn}>
            Import folder…
          </button>
          <button
            onClick={doWipe}
            disabled={busy !== null}
            style={{ ...btn, border: "1px solid var(--red)", color: "var(--red)" }}
          >
            Wipe corpus
          </button>
        </div>
        {report && (
          <div style={{ marginTop: 10, fontSize: 13 }}>
            <div style={{ color: "var(--green)" }}>imported: {report.imported}</div>
            {report.warnings.map((w, i) => (
              <div key={i} style={{ color: "var(--assembled)" }}>⚠ {w}</div>
            ))}
            {report.rejected.map((r, i) => (
              <div key={i} style={{ color: "var(--red)" }}>✕ {r}</div>
            ))}
          </div>
        )}
      </section>

      <section style={panel}>
        <h3 style={{ margin: "0 0 8px", fontSize: 13, color: "var(--text-muted)" }}>
          CORPUS — {cards.length} cards
        </h3>
        {cards.length === 0 && (
          <div style={{ color: "var(--text-dim)", fontSize: 14 }}>
            No cards yet. Anchor does not know anything until you teach it.
          </div>
        )}
        <div style={{ display: "grid", gap: 6 }}>
          {cards.map((c) => (
            <div
              key={c.id}
              style={{
                borderLeft: "3px solid var(--border)",
                paddingLeft: 10,
                fontSize: 14,
                display: "flex",
                alignItems: "baseline",
                gap: 8,
              }}
            >
              <span style={{ color: "var(--text)" }}>{c.title}</span>
              <span style={{ color: "var(--text-dim)", fontSize: 12, flex: 1 }}>
                {c.bullets.length} anchors · {c.language}
                {c.tags ? ` · ${c.tags}` : ""}
              </span>
              <button
                onClick={() => doDelete(c.id)}
                title="Delete this card"
                style={{
                  background: "none",
                  border: "none",
                  color: "var(--text-dim)",
                  cursor: "pointer",
                  fontSize: 13,
                }}
              >
                ✕
              </button>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

const btn: React.CSSProperties = {
  background: "var(--bg)",
  color: "var(--accent)",
  border: "1px solid var(--accent)",
  borderRadius: 6,
  padding: "8px 18px",
  fontSize: 14,
  cursor: "pointer",
};
