# Parent verification and preview scope

Requires Tezbar 0.3.0-beta.12 or later. The published app at the start of this work was beta.11; beta.12 app publication is a separate gate. This is a Rust development preview, not blanket upstream parity or live desktop/account acceptance.

Independent source review was completed before these repairs. Original freezes and their reports remain historical; the parent repaired source supersedes them. Builds ran locally on Windows and on an Apple Silicon Mac over Tailscale. No GitHub Actions builds or real service-account mutations were used. Source and immutable SDK provenance, original resources, Cargo.lock and dependency notices are retained.

The repaired source passed three Rust tests on each platform and real JSONL fixtures with 56 loopback GraphQL requests and ten synthetic mutations. Tray callbacks are bound to both menu generation and revision, including global read-all/disable actions; old callbacks cannot affect a re-enabled inbox. String-valued issue priority survives dependent field rerenders. Feature-free optimized binaries passed no-network production smoke; fixture builds are rejected by the packager.

User-configured personal API keys replace managed Raycast OAuth. Real account authorization, desktop UX, inbound tools/resources, attachments and the secondary workflows listed in PORT-REPORT.md remain unverified or unsupported. No full upstream parity claim is made.
