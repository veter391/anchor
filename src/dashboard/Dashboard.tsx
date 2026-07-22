import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface BootInfo {
  db_path: string;
  schema_version: number;
  tables: string[];
}

/** Phase-1 dashboard: proves the shell + DB boot. The real empty-state
 *  onboarding (Documents/06_DESIGN.md) lands with sessions in Phase 6. */
export function Dashboard() {
  const [boot, setBoot] = useState<BootInfo | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    invoke<BootInfo>("boot_info").then(setBoot).catch((e) => setErr(String(e)));
  }, []);

  return (
    <div
      style={{
        minHeight: "100vh",
        background: "var(--bg-elevated)",
        color: "var(--text)",
        padding: 32,
      }}
    >
      <h1 style={{ fontSize: 22, margin: 0 }}>
        ANCHOR{" "}
        <span style={{ color: "var(--text-muted)", fontSize: 14 }}>
          Prepared, not prompted.
        </span>
      </h1>

      <p style={{ color: "var(--text-muted)", maxWidth: 520 }}>
        Phase-1 skeleton. The overlay window shows a hardcoded card; drag it by
        its title row and place it under your webcam.
      </p>

      <h3 style={{ marginTop: 28, fontSize: 14, color: "var(--text-muted)" }}>
        LOCAL DATABASE
      </h3>
      {err && <pre style={{ color: "var(--red)" }}>{err}</pre>}
      {boot ? (
        <pre
          style={{
            background: "var(--bg)",
            border: "1px solid var(--border)",
            borderRadius: 8,
            padding: 14,
            fontSize: 13,
            fontFamily: "var(--font-mono)",
            overflowX: "auto",
          }}
        >
          {`path    : ${boot.db_path}\nschema  : v${boot.schema_version}\ntables  : ${boot.tables.join(", ")}`}
        </pre>
      ) : (
        !err && <p style={{ color: "var(--text-dim)" }}>loading…</p>
      )}
    </div>
  );
}
