# Suisei (彗星)

A native macOS GUI code editor with Xcode-grade design and behaviour.

Swift face over a Rust engine: the UI is SwiftUI/AppKit, all editor logic lives
in `suisei-core` (headless) and is bridged to Swift through the `suisei-engine`
C ABI. d (UI iteration — do NOT judge input latency on it)
SUISEI_FAST=1 ./scripts/run-suisei-app.sh

# Optimised build (default; ~4 min, the real thing)
./scripts/run-suisei-app.sh
```

In short:

- **Delayed public release only.** The developer publishes the previous year's
  final development snapshot (e.g. `2026dev`) when the next year version
  (e.g. `2027`) ships. Only that snapshot is covered by the Source License;
  current and shipping versions stay closed.
- **Personal, non-commercial use only.** Private forks, modifications, and
  builds are allowed. **Commercial Use — including internal use at a for-profit
  business — requires a separate written agreement.** Public distribution of
  forks is prohibited.
- **Attribution + licence integrity.** Forks 