//! About — what Anchor is and isn't, licence, and the consent note. Honest
//! by design (07_BRAND): the disqualifiers are stated plainly.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { panel, SectionTitle, Wordmark } from "../ui";

interface BootInfo {
  schema_version: number;
}

const IS = [
  "A recall aid: your prepared cards, surfaced the moment the topic comes up.",
  "A coverage tracker: what you have said, and what you still have not.",
  "Local-first: your notes and transcript never leave your machine.",
];
const ISNT = [
  "Not a stealth or undetectable mode — there is a screen-share exclusion toggle, off by default, and a Show-notes button.",
  "Not an answer generator to read aloud — bullets only, and machine-built ones are always marked.",
  "Not a substitute for knowing your subject.",
];

export function About() {
  const [boot, setBoot] = useState<BootInfo | null>(null);
  useEffect(() => {
    invoke<BootInfo>("boot_info").then(setBoot).catch(() => {});
  }, []);

  return (
    <div style={{ maxWidth: 760, margin: "0 auto", display: "grid", gap: 20 }}>
      <div>
        <Wordmark size={24} />
        <p style={{ color: "var(--text-muted)", fontSize: 14, margin: "10px 0 0" }}>
          Prepared, not prompted. Local-first live notes for online calls.
        </p>
      </div>

      <section style={panel}>
        <SectionTitle>What it is</SectionTitle>
        <ul style={{ margin: 0, paddingLeft: 18, color: "var(--text-soft)", fontSize: 14, display: "grid", gap: 6 }}>
          {IS.map((x, i) => (
            <li key={i}>{x}</li>
          ))}
        </ul>
      </section>

      <section style={panel}>
        <SectionTitle>What it is not</SectionTitle>
        <ul style={{ margin: 0, paddingLeft: 18, color: "var(--text-soft)", fontSize: 14, display: "grid", gap: 6 }}>
          {ISNT.map((x, i) => (
            <li key={i}>{x}</li>
          ))}
        </ul>
      </section>

      <section style={{ ...panel, background: "var(--bg-elevated)" }}>
        <SectionTitle>Consent &amp; recording</SectionTitle>
        <p style={{ color: "var(--text-muted)", fontSize: 13, margin: 0 }}>
          Anchor transcribes both sides of a call locally. In many places that requires the other
          party's consent, regardless of what the platform shows. You are responsible for
          obtaining consent where required; laws vary by jurisdiction. Best practice is to announce
          it at the start of the call.
        </p>
      </section>

      <div style={{ color: "var(--text-dim)", fontSize: 12 }}>
        Licence: AGPL-3.0 — use it, read it, fork it, but derivatives stay open.
        {boot ? ` · schema v${boot.schema_version}` : ""}
      </div>
    </div>
  );
}
