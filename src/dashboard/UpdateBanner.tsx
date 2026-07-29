//! In-app auto-update: on launch, quietly ask GitHub Releases (via the signed
//! updater manifest) whether a newer Anchor exists. If so, show a small banner
//! with an "Update now" button — one click downloads the new build (~17 MB; the
//! ~1 GB models stay in place), verifies its signature, installs, and relaunches.
//! Silent when there's nothing new, no endpoint yet, or the app is offline.

import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    let alive = true;
    check()
      .then((u) => {
        if (alive && u?.available) setUpdate(u);
      })
      .catch(() => {
        /* no endpoint / offline / nothing new — stay silent */
      });
    return () => {
      alive = false;
    };
  }, []);

  if (!update || dismissed) return null;

  const install = async () => {
    setErr(null);
    try {
      let total = 0;
      let got = 0;
      setBusy("Downloading…");
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") {
          total = e.data.contentLength ?? 0;
        } else if (e.event === "Progress") {
          got += e.data.chunkLength;
          setBusy(total ? `Downloading… ${Math.round((got / total) * 100)}%` : "Downloading…");
        } else if (e.event === "Finished") {
          setBusy("Installing…");
        }
      });
      setBusy("Restarting…");
      await relaunch();
    } catch (e) {
      setErr(`Update failed: ${String(e)}`);
      setBusy(null);
    }
  };

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "10px 16px",
        marginBottom: 22,
        background: "var(--accent-bg)",
        border: "1px solid var(--accent)",
        borderRadius: 10,
        fontSize: 13.5,
        color: "var(--text)",
      }}
    >
      <span aria-hidden style={{ fontSize: 15 }}>
        ✨
      </span>
      <span style={{ flex: 1 }}>
        {err ? (
          <span style={{ color: "var(--red)" }}>{err}</span>
        ) : (
          <>
            A new version of Anchor is available — <strong>{update.version}</strong>. Your cards,
            models and data stay as they are.
          </>
        )}
      </span>
      {busy ? (
        <span style={{ color: "var(--accent)", fontVariantNumeric: "tabular-nums" }}>{busy}</span>
      ) : (
        <>
          <button
            className="press"
            onClick={install}
            style={{
              background: "var(--accent)",
              color: "var(--bg)",
              border: "none",
              borderRadius: 7,
              padding: "6px 14px",
              fontSize: 13,
              fontWeight: 600,
              cursor: "pointer",
            }}
          >
            Update now
          </button>
          <button
            className="press"
            onClick={() => setDismissed(true)}
            style={{
              background: "transparent",
              color: "var(--text-muted)",
              border: "none",
              fontSize: 13,
              cursor: "pointer",
            }}
          >
            Later
          </button>
        </>
      )}
    </div>
  );
}
