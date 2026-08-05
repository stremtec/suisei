# Contributing to Suisei

> Suisei is **closed source** with a **1-year-delayed public release**. Read the
> three documents below before contributing:
>
> | Document | Applies to |
> |---|---|
> | [`LICENSE`](LICENSE) — Suisei Source License (`LicenseRef-Suisei`) | Delayed public source snapshots (e.g. `2026dev`) |
> | [`EULA`](EULA) — Suisei End User License Agreement | Shipping binaries (e.g. the `2027` `.app`) |
> | [`CLAs/SUISEI-CLA.md`](CLAs/SUISEI-CLA.md) — Suisei Contributor License Agreement | External contributions |

자세한 정책: [`docs/SUISEI-LICENSE-POLICY.md`](docs/SUISEI-LICENSE-POLICY.md)

## Contributions and the CLA

- **What can be contributed:** only the development of a **Released Version**
  (the public source snapshot, e.g. `2026dev`). Current development trees are
  not public.
- **CLA by default:** submitting any Contribution (pull request, patch,
  commit, issue-attached material) constitutes your agreement to the
  [Suisei CLA](CLAs/SUISEI-CLA.md). If the maintainer asks you to sign off on
  a pull request, that acknowledgement has the same effect.
- **Consequence:** your Contribution may be included in, and relicensed as
  part of, Suisei's **closed** products (e.g. `2027`) under terms the
  maintainer chooses, including proprietary/commercial terms. If that is not
  acceptable, do not contribute.

## Project conventions

- **Commit messages:** area prefixes are used, e.g.
  `core+engine+face: …`, `face: …`, `core: …`, `docs: …` — say what changed and
  why in the imperative.
- **Rust:** keep `cargo fmt` clean; run `cargo test --workspace` before
  submitting. Perf/benchmark tests are `#[ignore]`d by default and are not
  required for normal contributions.
- **FFI:** changes to the C ABI touch three coupled places —
  `suisei-engine/src/ffi.rs`, `suisei-engine/include/suisei_engine.h`, and the
  Swift decoder in `suisei-app/Suisei/EngineBridge.swift` — all covered by
  `suisei-engine/tests/abi_layout.rs`.
- **Licence notices:** mirrored licence text lives in `LICENSES/`
  (`LicenseRef-Suisei.txt`) for SPDX tooling — keep it in sync with `LICENSE`.
