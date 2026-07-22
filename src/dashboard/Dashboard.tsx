import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

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

  const refresh = useCallback(() => {
    invoke<BootInfo>("boot_info").then(setBoot).catch((e) => setErr(String(e)));
    invoke<CardRow[]>("list_cards").then(setCards).catch((e) => setErr(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    const t = setInterval(() => {
      // embedder_loaded flips asynchronously after the pre-warm finishes
      invoke<BootInfo>("boot_info").then(setBoot).catch(() => {});
    }, 2000);
    return () => clearInterval(t);
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
          <button
            onClick={async () => {
              await invoke("wipe_corpus");
              setReport(null);
              setResult(null);
              refresh();
            }}
            style={{ ...btn, borderColor: "var(--red)", color: "var(--red)" }}
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
              }}
            >
              <span style={{ color: "var(--text)" }}>{c.title}</span>{" "}
              <span style={{ color: "var(--text-dim)", fontSize: 12 }}>
                {c.bullets.length} anchors · {c.language}
                {c.tags ? ` · ${c.tags}` : ""}
              </span>
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
