//! Speech-model downloader — the first-run step that makes Anchor usable on a
//! fresh install. Mirrors the Mode-2 model picker (download / delete / live
//! progress) but for the on-device ASR bundles. Selection of which engine to
//! use lives in the Settings "Speech model" dropdown; this list just manages
//! what is installed on disk.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface AsrModelRow {
  id: string;
  name: string;
  tagline: string;
  languages: string;
  licence: string;
  size_bytes: number;
  installed: boolean;
}

const btn: React.CSSProperties = {
  background: "var(--bg)",
  color: "var(--accent)",
  border: "1px solid var(--accent)",
  borderRadius: 6,
  padding: "6px 14px",
  fontSize: 13,
  cursor: "pointer",
};

const fmtSize = (n: number) =>
  n >= 1_073_741_824 ? `${(n / 1_073_741_824).toFixed(1)} GB` : `${Math.round(n / 1_048_576)} MB`;

export function AsrModels() {
  const [models, setModels] = useState<AsrModelRow[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [err, setErr] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<AsrModelRow[]>("list_asr_models").then(setModels).catch((e) => setErr(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    const unP = listen<{ id: string; downloaded: number; total: number }>(
      "asr_model:progress",
      (e) =>
        setProgress((p) => ({
          ...p,
          [e.payload.id]: e.payload.total ? e.payload.downloaded / e.payload.total : 0,
        })),
    );
    const unD = listen<string>("asr_model:done", (e) => {
      setProgress((p) => {
        const n = { ...p };
        delete n[e.payload];
        return n;
      });
      refresh();
    });
    return () => {
      unP.then((f) => f()).catch(() => {});
      unD.then((f) => f()).catch(() => {});
    };
  }, [refresh]);

  const download = (id: string) => {
    setErr(null);
    setProgress((p) => ({ ...p, [id]: 0 }));
    invoke("download_asr_model", { id }).catch((e) => {
      setErr(String(e));
      setProgress((p) => {
        const n = { ...p };
        delete n[id];
        return n;
      });
    });
  };
  const remove = async (id: string) => {
    await invoke("delete_asr_model", { id }).catch((e) => setErr(String(e)));
    refresh();
  };

  const noneInstalled = models.length > 0 && models.every((m) => !m.installed);

  return (
    <div style={{ display: "grid", gap: 8 }}>
      {err && <div style={{ color: "var(--red)", fontSize: 12 }}>{err}</div>}
      {noneInstalled && (
        <div style={{ color: "var(--assembled)", fontSize: 12.5, lineHeight: 1.5 }}>
          No speech model yet — download one below before your first call. Anchor needs it to hear
          the conversation.
        </div>
      )}
      {models.map((m) => {
        const dl = progress[m.id];
        const downloading = dl !== undefined;
        return (
          <div
            key={m.id}
            style={{
              border: `1px solid ${m.installed ? "var(--accent)" : "var(--border)"}`,
              borderRadius: 6,
              padding: "8px 12px",
              display: "flex",
              alignItems: "center",
              gap: 10,
            }}
          >
            <div style={{ flex: 1 }}>
              <span style={{ color: "var(--text)", fontSize: 14 }}>{m.name}</span>{" "}
              <span style={{ color: "var(--accent)", fontSize: 12 }}>· {m.tagline}</span>
              <div style={{ color: "var(--text-dim)", fontSize: 11 }}>
                {m.languages} · {fmtSize(m.size_bytes)} · {m.licence}
              </div>
            </div>
            {downloading ? (
              <span style={{ color: "var(--assembled)", fontSize: 12 }}>{Math.round(dl * 100)}%</span>
            ) : m.installed ? (
              <>
                <span style={{ color: "var(--accent)", fontSize: 12 }}>installed</span>
                <button
                  onClick={() => remove(m.id)}
                  style={{ ...btn, border: "1px solid var(--border)", color: "var(--text-dim)" }}
                >
                  Delete
                </button>
              </>
            ) : (
              <button onClick={() => download(m.id)} style={btn}>
                Download
              </button>
            )}
          </div>
        );
      })}
      <p style={{ color: "var(--text-dim)", fontSize: 11, margin: 0 }}>
        Downloaded once from Hugging Face into Anchor&apos;s own folder, then cached offline. The
        dropdown above chooses which one a call uses.
      </p>
    </div>
  );
}
