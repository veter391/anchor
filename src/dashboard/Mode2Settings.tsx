import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface ModelRow {
  id: string;
  name: string;
  tagline: string;
  size_bytes: number;
  licence: string;
  is_default: boolean;
  installed: boolean;
}

interface LlmConfig {
  mode: string;
  local_model: string;
  api_provider: string;
  api_model: string | null;
  api_key_set: boolean;
}

const panel: React.CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border)",
  borderRadius: 8,
  padding: 14,
};
const btn: React.CSSProperties = {
  background: "var(--bg)",
  color: "var(--accent)",
  border: "1px solid var(--accent)",
  borderRadius: 6,
  padding: "6px 14px",
  fontSize: 13,
  cursor: "pointer",
};
const gb = (n: number) => `${(n / 1073741824).toFixed(1)} GB`;

/** Mode-2 provider settings: the free local models (download/switch, Handy-style)
 *  or a bring-your-own-key API provider. */
export function Mode2Settings() {
  const [cfg, setCfg] = useState<LlmConfig | null>(null);
  const [models, setModels] = useState<ModelRow[]>([]);
  const [progress, setProgress] = useState<Record<string, number>>({});
  const [apiKey, setApiKey] = useState("");
  const [err, setErr] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<LlmConfig>("get_llm_config").then(setCfg).catch((e) => setErr(String(e)));
    invoke<ModelRow[]>("list_models").then(setModels).catch((e) => setErr(String(e)));
  }, []);

  useEffect(() => {
    refresh();
    const unP = listen<{ id: string; downloaded: number; total: number }>(
      "model:progress",
      (e) =>
        setProgress((p) => ({
          ...p,
          [e.payload.id]: e.payload.total ? e.payload.downloaded / e.payload.total : 0,
        })),
    );
    const unD = listen<string>("model:done", (e) => {
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

  const setMode = async (mode: string) => {
    await invoke("set_llm_config", { mode }).catch((e) => setErr(String(e)));
    refresh();
  };
  const pickLocal = async (id: string) => {
    await invoke("set_llm_config", { localModel: id }).catch((e) => setErr(String(e)));
    refresh();
  };
  const download = (id: string) => {
    setProgress((p) => ({ ...p, [id]: 0 }));
    invoke("download_model", { id }).catch((e) => {
      setErr(String(e));
      setProgress((p) => {
        const n = { ...p };
        delete n[id];
        return n;
      });
    });
  };
  const remove = async (id: string) => {
    await invoke("delete_model", { id }).catch((e) => setErr(String(e)));
    refresh();
  };
  const saveKey = async () => {
    if (!cfg) return;
    await invoke("set_api_key", { provider: cfg.api_provider, key: apiKey }).catch((e) =>
      setErr(String(e)),
    );
    setApiKey("");
    refresh();
  };

  if (!cfg) return null;

  return (
    <section style={panel}>
      <h3 style={{ margin: "0 0 4px", fontSize: 13, color: "var(--text-muted)" }}>
        UNEXPECTED-QUESTION ENGINE (Mode 2)
      </h3>
      <p style={{ color: "var(--text-muted)", fontSize: 12, marginTop: 0 }}>
        When a question you didn&apos;t prepare comes up, Anchor assembles a card from
        your material. Run it free on your machine, or bring your own API key.
      </p>
      {err && <div style={{ color: "var(--red)", fontSize: 12 }}>{err}</div>}

      <div style={{ display: "flex", gap: 8, margin: "8px 0" }}>
        {["local", "api"].map((m) => (
          <button
            key={m}
            onClick={() => setMode(m)}
            style={{
              ...btn,
              border: `1px solid ${cfg.mode === m ? "var(--accent)" : "var(--border)"}`,
              color: cfg.mode === m ? "var(--accent)" : "var(--text-muted)",
            }}
          >
            {m === "local" ? "● Free — on your machine" : "○ API key (fastest)"}
          </button>
        ))}
      </div>

      {cfg.mode === "local" && (
        <div style={{ display: "grid", gap: 8 }}>
          {models.map((m) => {
            const dl = progress[m.id];
            const downloading = dl !== undefined;
            const active = cfg.local_model === m.id;
            return (
              <div
                key={m.id}
                style={{
                  border: `1px solid ${active && m.installed ? "var(--accent)" : "var(--border)"}`,
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
                    {gb(m.size_bytes)} · {m.licence}
                    {m.is_default ? " · recommended" : ""}
                  </div>
                </div>
                {downloading ? (
                  <span style={{ color: "var(--assembled)", fontSize: 12 }}>
                    {Math.round(dl * 100)}%
                  </span>
                ) : m.installed ? (
                  <>
                    {active ? (
                      <span style={{ color: "var(--accent)", fontSize: 12 }}>active</span>
                    ) : (
                      <button onClick={() => pickLocal(m.id)} style={btn}>
                        Use
                      </button>
                    )}
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
            First download picks the model up from Hugging Face into Anchor&apos;s own
            folder. You can install all three and switch anytime.
          </p>
        </div>
      )}

      {cfg.mode === "api" && (
        <div style={{ display: "grid", gap: 8 }}>
          <label style={{ color: "var(--text-muted)", fontSize: 12 }}>
            Provider
            <select
              value={cfg.api_provider}
              onChange={(e) => invoke("set_llm_config", { apiProvider: e.target.value }).then(refresh)}
              style={{
                marginLeft: 8,
                background: "var(--bg-elevated)",
                color: "var(--text)",
                colorScheme: "dark",
                border: "1px solid var(--border)",
                borderRadius: 6,
                padding: "4px 8px",
              }}
            >
              <option style={{ background: "#161b22", color: "#e8edf2" }} value="openrouter">
                OpenRouter (any model, one key)
              </option>
              <option style={{ background: "#161b22", color: "#e8edf2" }} value="groq">
                Groq
              </option>
              <option style={{ background: "#161b22", color: "#e8edf2" }} value="openai">
                OpenAI
              </option>
              <option style={{ background: "#161b22", color: "#e8edf2" }} value="custom">
                Custom (OpenAI-compatible)
              </option>
            </select>
          </label>
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={cfg.api_key_set ? "key saved — enter to replace" : "paste API key"}
              style={{
                flex: 1,
                background: "var(--bg-elevated)",
                border: "1px solid var(--border)",
                borderRadius: 6,
                color: "var(--text)",
                padding: "8px 10px",
                fontSize: 13,
              }}
            />
            <button onClick={saveKey} style={btn}>
              Save key
            </button>
          </div>
          <p style={{ color: "var(--text-dim)", fontSize: 11, margin: 0 }}>
            Keys are stored in your OS credential manager, never in Anchor&apos;s files.
            {cfg.api_key_set ? " A key is currently saved for this provider." : ""}
          </p>
        </div>
      )}
    </section>
  );
}
