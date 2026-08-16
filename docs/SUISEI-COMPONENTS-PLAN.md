# Downloadable components

User's ask: Xcode's Components/Platforms model — ship the editor, download the
rest. Chosen at install, changeable later. "번들은 너가 구분해서 한번 해봐. 뭐
fbx, debugger 등등등."

Written 2026-08-17. The split below is measured, and the measurement moves the
answer a long way from where the question starts.

---

## 0. Where the weight actually is

`suisei-app/.build/Suisei.app`, **140 MB**.

| | size | where |
|---|---|---|
| **Language grammars** | **~38 MB** | `__const` in `libsuisei_engine.dylib` |
| Welcome hero art | 19 MB | `Resources/WelcomeHeroes/*.jpg`, 10 files |
| Assimp (FBX) | 6.0 MB | **statically linked into the executable** |
| GLTFKit2 (glTF/GLB) | 5.4 MB | `Frameworks/`, already a dynamic framework |
| Engine code proper | 5.3 MB | `__text` in the same dylib |
| **Debugger** | **0 MB** | uses the system's `lldb-dap`; we ship nothing |

The engine dylib is 43 MB and only **5.3 MB of it is code**. `__const` is 38.8 MB,
and that is thirty tree-sitter grammars' parse tables:

```
objc 5.4 · c_sharp 5.4 · swift 5.0 · scala 4.3 · haskell 3.9 · cpp 3.6
typescript 3.2 · php 2.5 · ruby 2.2 · dart 2.0 · elixir 1.5 · bash 1.4
rust 1.3 · zig 0.8 · md 0.8 · c 0.8 · python 0.6 · java 0.6 · …
```

**The executable's 67 MB is not a shipping number** — that is the `-Onone`
development build this was measured from, unstripped. Do not quote it.

So the question "which bundles" has an answer the question did not expect:
**languages are the product's weight, by a factor of six over everything else
combined that is separable.** FBX is 6 MB. The debugger is nothing at all.

---

## 1. The debugger is not a bundle

Today's bug was that Suisei said "install lldb-dap" on a Mac that had it twice
over — the gate walked `PATH`, and Apple's toolchain is on nobody's `PATH`.
Fixed. The lesson generalises:

- **Rust / C / C++** → `lldb-dap`, ships inside Xcode and inside the Command
  Line Tools. Hosting our own copy would mean shipping a second debugger next
  to the one Apple already updates.
- **Python / Go / Node** → `debugpy`, `dlv`, `js-debug-adapter`. These come from
  pip / go install / npm, which is where their users already get everything else
  and where the security updates come from.

So the component for debugging is not a binary. It is **finding what is there,
and helping install what is not** — a row that says "Python debugging: not
installed · `pip install debugpy` · Copy", and that turns into "installed at
/opt/homebrew/bin/…" once it is. Hosting and signing other people's debuggers
buys nothing and takes on their CVEs.

The same argument covers LSP servers.

---

## 2. Languages are the bundle

Thirty grammars, ~38 MB, and it is the one axis where a user's own answer is
obvious: nobody writes all thirty. It also splits cleanly — a grammar is an
independent parse table with one entry point.

**Ship in the app** (small, and what a first launch has to work without a
network): rust, python, javascript, typescript, json, markdown, toml, yaml,
html, css, c, bash, go, java. Together well under 10 MB.

**Downloadable**: objc, c_sharp, swift, scala, haskell, cpp, php, ruby, dart,
elixir, zig, lua, xml, cmake, and the rest.

That is roughly **28 MB out of the box**, with the common case still fully
offline.

Swift at 5.0 MB is the judgement call: Suisei is a Mac editor and Swift may be a
daily language for its users, which argues for shipping it despite the size.
That is a product decision, not a technical one — the table above is what it
should be decided from.

### What this costs, technically

Grammars are compiled-in Rust crates today (`suisei-core/Cargo.toml`, 33
`tree-sitter-*` dependencies). A downloadable grammar must be loadable at
runtime, which means **a dylib per language with a `tree_sitter_<lang>()`
entry point**, `dlopen`ed on demand. That is how Zed and nvim-treesitter do it,
so the shape is proven; it is still a real change to how the engine gets its
languages.

---

## 3. The 3D viewer is a real bundle, and it is blocked

GLTFKit2 (5.4 MB) is already a dynamic framework and could be weak-linked
today. **Assimp (6.0 MB) is a static archive linked into the executable**, so
it cannot be removed without becoming a dylib first. 11.4 MB together.

Degrading is already correct here and needs no new design: before Assimp, an
FBX landed on `FilePlaceholderView`, which names the file and offers to open it
elsewhere. A machine without the 3D component gets exactly that back.

---

## 4. The constraint that decides the schedule

**Anything downloaded and loaded into the process must be signed and
notarized.** The hardened runtime's library validation refuses to load a dylib
that is not signed by the same team, and turning library validation off to
dodge that would trade the app's security posture for a download.

Three ways to live with it:

- **(a) Sign and notarize our own components.** Correct, and it means a build,
  signing, notarization and hosting pipeline that does not exist yet. This is
  the real cost of the whole feature — far more than the code that lists and
  downloads them.
- **(b) Disable library validation.** No.
- **(c) Out of process.** Suisei already ships helpers (`Contents/Helpers`: the
  daemon and the menu-bar agent), and a helper does not face library validation
  the same way. Viable for a 3D importer, where one conversion per file opened
  is fine. **Not** viable for grammars: syntax parses on every keystroke, and an
  RPC per keystroke is exactly the budget this editor spends its time
  protecting.

So: (a) for grammars, (a) or (c) for 3D. And (a) is on the critical path either
way — it should be built and proved with ONE small component before thirty
grammars depend on it.

---

## 5. A rule for the whole feature

**A missing component degrades; it never breaks.** An `.fbx` with no 3D
component is a named binary file, not an error. A language with no grammar is
an editable document without highlighting, not a refusal to open. Core already
behaves this way for an unknown extension, so this is a property to preserve
rather than to invent — and it is what makes "download later" safe to offer at
all.

Corollary: the app must never *require* a download to do something it could do
without one. A first launch with no network is a working editor.

---

## 6. Staging

**C0 — the 19 MB of jpegs.** Ten hero images at full resolution in
`Resources`. No architecture, no download, no UI: re-encode to the size they are
actually drawn at. The single cheapest 15 MB in the app and it should not wait
behind any of this.

**C1 — the pipeline, with one component.** Build, sign, notarize and host ONE
grammar; teach the engine to `dlopen` it; install and remove it by hand. Nothing
user-facing. This is where the feature's real risk is, and it is worth paying
before anything depends on it.

**C2 — grammars as components.** The rest of the thirty, and the shipped set
decided from §2.

**C3 — the Components UI.** Settings → Components: name, size, installed or
not, install/remove, progress. Plus a first-run picker, which is Xcode's
question at install time. The UI is the easy part and it comes last on purpose.

**C4 — tooling rows.** Debug adapters and language servers as detected-and-
helped, per §1. No hosting.

**C5 — 3D as a component.** Assimp to a dylib, GLTFKit2 weak-linked, and the
placeholder path re-confirmed as the degraded case.

---

## 7. The honest summary

The user's list was "fbx, debugger 등등등". Measured: the debugger is 0 MB and
was a lookup bug, FBX is 6 MB, and **languages are 38 MB** — six times
everything else that can be split, put together. If only one component ever
ships, it should be a language pack.

And the largest single cost of the feature is not the downloading. It is the
signing and notarization pipeline that has to exist before a downloaded dylib
can be loaded into a hardened process at all.
