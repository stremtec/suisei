# Suisei (彗星)

A native macOS GUI code editor with Xcode-grade design and behaviour.

Swift face over a Rust engine: the UI is SwiftUI/AppKit, all editor logic lives
in `suisei-core` (headless) and is bridged to Swift through the `suisei-engine`
C ABI. d (UI iteration — do NOT judge input latency on it)
SUISEI_FAST=1 ./scripts/run-suisei-app.sh

# Optimised build (default; ~4 min, the real thing)
./scripts/run-suisei-app.sh
```

The Rust
- `docs/SUISEI-TODO.md` — open bugs and the hard-won design notes (metaball
 ** and is **not** an open-source project. The
licensing regime has three layers:

- **Suisei Source License** — [`LICENSE`](LICENSE) (`LicenseRef-Suisei`):
  applies to the delayed public source snapshots only (e.g. `2026dev`).
- **Suisei EULA** — [`EULA`](EULA): applies to shipping binaries.
- **Suisei CLA** — [`CLAs/SUISEI-CLA.md`](CLAs/SUISEI-CLA.md): applies to
  external contributions.

In short:

- **Delayed public release only.** The developer publishes the previous year's
  final development snapshot (e.g. `2026dev`) when the next year version
  (e.g. `2027`) ships. Only that snapshot is covered by the Source License;
  current and shipping versions stay closed.
- **Personal, non-commercial use only.** Private forks, modifications, and
  builds are allowed. **Commercial Use — including internal use at a for-profit
  business — requires a separate written agreement.** Public distribution of
  forks is prohibited.
- **Attribution + licence integrity.** Forks must state that they are derived
  from Suisei, keep the copyright notice, and carry `LICENSE` verbatim;
  changing or replacing the licence terms is prohibited.
- **User works and plugins are free.** Code you write or build with Suisei, and
  plugins/extensions built against its public interfaces, are not covered by
  the Suisei licence.
- **No source access or reverse engineering** of unreleased versions, except
  to the extent applicable law requires.

Full policy (한국어): [`docs/SUISEI-LICENSE-POLICY.md`](docs/SUISEI-LICENSE-POLICY.md)
Contribution guide: [`CONTRIBUTING.md`](CONTRIBUTING.md)
