# Code signing

Anchor's Windows builds are currently **unsigned**, so SmartScreen shows a
"Windows protected your PC" warning on first run (see
[distribution.md](distribution.md)). Signing removes that warning. The release
pipeline ([`.github/workflows/release.yml`](../.github/workflows/release.yml)) is
already wired to sign automatically — it is **off until the signing secrets
exist**, and turns on the moment they do. No code change is needed to enable it.

Signing requires a certificate, and getting one is a human, paid, identity-
verified process that cannot be automated. The recommended route is **Azure
Trusted Signing** (~$10/month, no hardware token, CI-native). These are the
one-time owner steps.

## One-time setup (owner)

1. **Azure account.** Sign in at [portal.azure.com](https://portal.azure.com)
   with a Microsoft account and add a payment method (a Pay-As-You-Go
   subscription). Trusted Signing is ~$9.99/month.
2. **Create a Trusted Signing Account.** Search "Trusted Signing Accounts" →
   Create. Pick a region — its endpoint (e.g. `https://eus.codesigning.azure.net`)
   is one of the secrets below. Note the **account name**.
3. **Identity validation.** Under the account, start an **Identity Validation**
   (Individual or Organization). Microsoft verifies your legal identity; this
   takes ~1–7 business days and needs a government ID. The verified name becomes
   the certificate's subject.
4. **Certificate profile.** Once validation is approved, create a **Certificate
   Profile** of type **Public Trust**. Note the **profile name**.
5. **Service principal for CI.** In Entra ID → App registrations → New
   registration. Create a **client secret**. Then, on the Trusted Signing
   account's Access control (IAM), assign this app the role **Trusted Signing
   Certificate Profile Signer**.
6. **Add the GitHub repo secrets** (repo → Settings → Secrets and variables →
   Actions):

   | Secret | Value |
   |---|---|
   | `AZURE_TENANT_ID` | Entra tenant (Directory) ID |
   | `AZURE_CLIENT_ID` | the app registration's Application (client) ID |
   | `AZURE_CLIENT_SECRET` | the client secret value |
   | `AZURE_SIGNING_ENDPOINT` | e.g. `https://eus.codesigning.azure.net` |
   | `AZURE_SIGNING_ACCOUNT` | the Trusted Signing account name |
   | `AZURE_SIGNING_PROFILE` | the certificate profile name |

That's it. The next tagged build (`git tag v0.6.1 && git push --tags`, or a
`workflow_dispatch` run) will sign the installer, and the SmartScreen warning
goes away for anyone who downloads it.

## Verifying it worked

After a signed run, download the `-setup.exe` from the release/artifact and check
`Properties → Digital Signatures` — it should list your verified identity. Or run
`signtool verify /pa Anchor_*-setup.exe`.

## Notes / future

- The workflow signs the **installer** (`*-setup.exe`), which is what SmartScreen
  checks on download. Signing the inner `anchor.exe` as well (a sign step between
  the release build and the bundle) is a later enhancement for defense in depth.
- Keep `AZURE_CLIENT_SECRET` fresh — Entra client secrets expire (set a reminder).
- If you ever move off Azure Trusted Signing, the only change is the "Sign
  installer" step in the release workflow; the rest of the pipeline is unchanged.
