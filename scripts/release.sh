#!/usr/bin/env bash
#
# One command from a checkout to a .dmg somebody else can open.
#
#   ./scripts/release.sh
#
# Does, in order: shrink the welcome art · build the engine and the face at -O ·
# package Suisei.app · lay out and compress a .dmg · print what the person on
# the other end has to do.
#
# ── The one thing that is not normal about this release ──────────────────────
#
# It is UNSIGNED. A free Apple account has neither a Developer ID certificate
# nor notarization, and those are the two things Gatekeeper looks for. The
# receiver goes to System Settings → Privacy & Security → "Open Anyway", once.
# From Sequoia on, the old right-click-Open bypass is gone, so that panel is
# the only route and the .dmg says so in writing.
#
# Signing is a seam, not a rewrite: set SUISEI_SIGN_ID and this script signs and
# staples. Everything else about the build is identical either way — which is
# the point of having the seam now rather than discovering it later.
#
#   SUISEI_SIGN_ID="Developer ID Application: NAME (TEAMID)" ./scripts/release.sh
#
# ── Options ──────────────────────────────────────────────────────────────────
#
#   SUISEI_SIGN_ID=…    sign and notarize (needs a paid account)
#   SUISEI_KEEP_ART=1   ship the full-size masters (skip the downscale)
#   SUISEI_HERO_MAX=…   long edge in px for the downscale (default 1800)
#   SUISEI_HERO_Q=…     JPEG quality 1-100 (default 80)
#   SUISEI_SKIP_TESTS=1 do not run the test suite first
#   SUISEI_NO_DMG=1     stop after the .app — what the in-app updater builds,
#                       which replaces a bundle rather than shipping an image
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

BUILD="$ROOT/suisei-app/.build"
APP="$BUILD/Suisei.app"
ART_SRC="$ROOT/suisei-app/Resources/WelcomeHeroes"
ART_OUT="$BUILD/heroes"
DIST="$ROOT/dist"

HERO_MAX="${SUISEI_HERO_MAX:-1800}"
HERO_Q="${SUISEI_HERO_Q:-80}"

bold()  { printf '\033[1m%s\033[0m\n' "$*"; }
step()  { printf '\n\033[1;34m▸ %s\033[0m\n' "$*"; }
note()  { printf '  %s\n' "$*"; }
die()   { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }
# `die` with the tool's own words. A quiet command behind `|| die` reports that
# something failed and destroys the only sentence that said what — which is
# exactly how the first run of this script lost `hdiutil`'s reason.
run()   { local what="$1"; shift; local out
          if ! out="$("$@" 2>&1)"; then
            printf '\033[1;31m✗ %s\033[0m\n' "$what" >&2
            [[ -n "$out" ]] && printf '  %s\n' "$out" >&2
            exit 1
          fi; }

human() { du -sh "$1" 2>/dev/null | cut -f1; }

# ─────────────────────────────────────────────────────────────────────────────
step "Preflight"

for tool in cargo swiftc sips hdiutil; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool not found"
done

# A release build needs room for `target/release` plus the .app plus an
# uncompressed .dmg staging copy. Running out halfway leaves a half-linked
# binary that looks like a compiler bug, which is a bad afternoon; this is one
# `df` call to not have it.
FREE_GB=$(df -g "$ROOT" | awk 'NR==2 {print $4}')
note "free disk: ${FREE_GB}G"
[[ "$FREE_GB" -ge 12 ]] || die "need ~12G free for a release build, have ${FREE_GB}G"

VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | cut -d'"' -f2)"
[[ -n "$VERSION" ]] || die "no version in Cargo.toml"
note "version: $VERSION"

if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
  note "⚠ working tree is dirty — this build will not match any commit"
fi
COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
note "commit:  $COMMIT"

if [[ -n "${SUISEI_SIGN_ID:-}" ]]; then
  note "signing: $SUISEI_SIGN_ID"
else
  note "signing: none — the .dmg will carry first-run instructions"
fi

# ─────────────────────────────────────────────────────────────────────────────
if [[ "${SUISEI_SKIP_TESTS:-0}" != "1" ]]; then
  step "Tests"
  # Before the slow part, not after. A release build is ~15 minutes and there is
  # no reason to spend it to then find out the engine is broken.
  cargo test -p suisei-core -p suisei-engine --quiet 2>&1 | tail -3
  note "core + engine green"
fi

# ─────────────────────────────────────────────────────────────────────────────
step "Welcome art"

if [[ "${SUISEI_KEEP_ART:-0}" == "1" ]]; then
  note "SUISEI_KEEP_ART=1 — shipping the masters at full size"
  unset SUISEI_HERO_DIR || true
elif [[ ! -d "$ART_SRC" ]]; then
  note "no art directory — nothing to do"
else
  # The panel is 516×500pt and macOS tops out at 2x, so the largest the art is
  # ever drawn is ~1732px on its long edge. The masters are 3200px. Every one of
  # those extra pixels rides in every download and is then thrown away by the
  # scaler on the way to the screen.
  #
  # Output goes to .build, never back over the source. These are the masters;
  # a lossy pass written over its own input loses a little more every run.
  mkdir -p "$ART_OUT"
  changed=0
  for f in "$ART_SRC"/*.{jpg,jpeg,png}; do
    [[ -e "$f" ]] || continue
    out="$ART_OUT/$(basename "$f")"
    # Skip what is already current — a second release should not re-encode.
    if [[ -f "$out" && "$out" -nt "$f" ]]; then continue; fi
    sips -Z "$HERO_MAX" -s format jpeg -s formatOptions "$HERO_Q" \
         "$f" --out "$out" >/dev/null 2>&1 || die "sips failed on $(basename "$f")"
    changed=$((changed + 1))
  done
  if [[ "$changed" -gt 0 ]]; then
    note "re-encoded $changed image(s) at ${HERO_MAX}px / q${HERO_Q}"
  else
    note "already current"
  fi
  note "$(human "$ART_SRC") masters → $(human "$ART_OUT") shipped"
  export SUISEI_HERO_DIR="$ART_OUT"
fi

# ─────────────────────────────────────────────────────────────────────────────
step "Build"
note "engine + face at -O (this is the slow one)"
SUISEI_PROFILE=release "$ROOT/scripts/package-suisei-app.sh"
[[ -d "$APP" ]] || die "packaging produced no .app"
note "app: $(human "$APP")"

# ─────────────────────────────────────────────────────────────────────────────
step "Sign"
if [[ -n "${SUISEI_SIGN_ID:-}" ]]; then
  # --deep is deprecated and does the wrong thing on nested code; sign the
  # inner pieces first, then the bundle, which is what Apple documents.
  find "$APP/Contents/Frameworks" "$APP/Contents/Helpers" -type f -perm +111 2>/dev/null |
    while read -r bin; do
      codesign --force --options runtime --timestamp --sign "$SUISEI_SIGN_ID" "$bin"
    done
  ENT="$ROOT/suisei-app/Suisei.entitlements"
  if [[ -f "$ENT" ]]; then
    codesign --force --options runtime --timestamp \
             --entitlements "$ENT" --sign "$SUISEI_SIGN_ID" "$APP"
  else
    codesign --force --options runtime --timestamp --sign "$SUISEI_SIGN_ID" "$APP"
  fi
  codesign --verify --strict --verbose=2 "$APP"
  note "signed and verified"
else
  # An ad-hoc signature is not a Developer ID and Gatekeeper still stops it.
  # It is worth doing anyway: without ANY signature the dylib and the helpers
  # can be rejected outright on arm64 rather than merely warned about.
  codesign --force --sign - "$APP" 2>/dev/null || true
  note "ad-hoc signed (Gatekeeper will still ask; see FIRST-RUN.txt)"
fi

# ─────────────────────────────────────────────────────────────────────────────
if [[ "${SUISEI_NO_DMG:-0}" == "1" ]]; then
  step "Done"
  bold "  $APP"
  note "size: $(human "$APP")"
  note "no disk image — SUISEI_NO_DMG=1"
  exit 0
fi

step "Disk image"

mkdir -p "$DIST"
DMG="$DIST/Suisei-$VERSION.dmg"
STAGE="$BUILD/dmg-stage"
VOLNAME="Suisei $VERSION"

rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/Suisei.app"
ln -s /Applications "$STAGE/Applications"

# The instructions ride INSIDE the image, because that is the one place the
# receiver is certainly looking. A link in a chat message is not.
cat > "$STAGE/FIRST-RUN.txt" <<TXT
Suisei $VERSION  ($COMMIT)

INSTALL
  Drag Suisei onto the Applications folder beside it.

FIRST LAUNCH
  This build is not signed with an Apple Developer ID, so macOS will refuse
  to open it the first time and say it "cannot be verified".

  That message is not a virus warning. It means Apple has not been paid to
  vouch for the build. To open it anyway:

    1. Double-click Suisei. Read the message, click Done.
    2. Open System Settings → Privacy & Security.
    3. Scroll down. There is a line about Suisei being blocked, with an
       "Open Anyway" button. Click it.
    4. Double-click Suisei again and click Open.

  Once only. Every launch after that is normal.

  (On macOS Sequoia and later, right-click → Open no longer works for this.
  The Privacy & Security panel is the only way.)

IF IT SAYS "DAMAGED AND CAN'T BE OPENED"
  Nothing is damaged. That wording appears instead of the one above when the
  download was tagged by the browser that fetched it, and there is no
  Developer ID signature to check the tag against. There is no "Open Anyway"
  button in this case — the tag has to come off first.

  With Suisei already in Applications, open Terminal and run:

      xattr -dr com.apple.quarantine /Applications/Suisei.app

  Then open Suisei normally. Once only, same as above.

IF YOU WOULD RATHER NOT
  That is a reasonable thing to decide. Nothing here needs you to trust a
  binary — the source is in the repository and builds with:

      ./scripts/release.sh
TXT

# Optional polish: a background and placed icons. Guarded, because it drives
# Finder through AppleScript, which needs Automation permission and is not
# available headless — and a release must not fail over the wallpaper.
LAYOUT_OK=0
DMG_RW="$BUILD/Suisei-rw.dmg"
rm -f "$DMG_RW"
# No `-fs`. Naming one is what broke the first run of this script: `-fs HFS+`
# fails on this machine with "no mountable file systems", and so does JHFS+.
# hdiutil picks a filesystem the host can actually create, and the app already
# requires a macOS far newer than the HFS+-only ones.
run "hdiutil create failed" \
  hdiutil create -srcfolder "$STAGE" -volname "$VOLNAME" -format UDRW -ov -quiet "$DMG_RW"

# Mounted at /Volumes, not a temp dir: the AppleScript below addresses the
# volume by NAME, and Finder cannot find one mounted anywhere else.
# A volume left mounted by an earlier run holds the image open, and the convert
# below then fails with the source busy. That happened: a detach failed, the
# `|| true` after it swallowed the failure, and the NEXT release died at its last
# step with a cause two runs upstream. Clear ours before starting rather than
# trusting the cleanup to have worked.
for stale in /Volumes/"$VOLNAME"*; do
  [[ -d "$stale" ]] && hdiutil detach "$stale" -force -quiet 2>/dev/null || true
done

MOUNT="/Volumes/$VOLNAME"
if hdiutil attach "$DMG_RW" -quiet 2>/dev/null && [[ -d "$MOUNT" ]]; then
  if osascript >/dev/null 2>&1 <<APPLESCRIPT
tell application "Finder"
  tell disk "$VOLNAME"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {200, 160, 800, 560}
    set opts to the icon view options of container window
    set arrangement of opts to not arranged
    set icon size of opts to 96
    set position of item "Suisei.app" of container window to {150, 190}
    set position of item "Applications" of container window to {450, 190}
    set position of item "FIRST-RUN.txt" of container window to {300, 330}
    update without registering applications
    close
  end tell
end tell
APPLESCRIPT
  then
    LAYOUT_OK=1
    sync
  fi
  # Finder can still be holding the volume for a moment after `close`. Retry
  # before forcing: an image left attached is what breaks the NEXT release, not
  # this one, which is the hardest kind of failure to trace back.
  for attempt in 1 2 3; do
    hdiutil detach "$MOUNT" -quiet 2>/dev/null && break
    sleep 1
    if [[ "$attempt" == 3 ]]; then
      hdiutil detach "$MOUNT" -force -quiet 2>/dev/null || true
    fi
  done
fi

if [[ "$LAYOUT_OK" == "1" ]]; then
  note "window laid out"
else
  note "⚠ Finder layout skipped (needs Automation permission) — icons unplaced"
fi

# UDZO: compressed and read-only, which is what a download should be.
# No `-quiet`: `run` exists to keep the tool's own words for the failure
# message, and `-quiet` is exactly what throws them away. The first time this
# step failed it reported nothing but its own name.
run "hdiutil convert failed" \
  hdiutil convert "$DMG_RW" -format UDZO -imagekey zlib-level=9 -ov -o "$DMG"
rm -f "$DMG_RW"
rm -rf "$STAGE"

# ─────────────────────────────────────────────────────────────────────────────
step "Notarize"
if [[ -n "${SUISEI_SIGN_ID:-}" ]]; then
  codesign --force --sign "$SUISEI_SIGN_ID" "$DMG"
  # Needs `xcrun notarytool store-credentials suisei` to have been run once.
  xcrun notarytool submit "$DMG" --keychain-profile suisei --wait \
    || die "notarization failed"
  xcrun stapler staple "$DMG" || die "stapling failed"
  note "notarized and stapled"
else
  note "skipped — no Developer ID"
fi

# ─────────────────────────────────────────────────────────────────────────────
step "Done"
bold "  $DMG"
note "size:   $(human "$DMG")"
note "sha256: $(shasum -a 256 "$DMG" | cut -d' ' -f1)"
note "app:    $(human "$APP")"
echo
if [[ -z "${SUISEI_SIGN_ID:-}" ]]; then
  note "This build is unsigned. Whoever you send it to opens it once through"
  note "System Settings → Privacy & Security → Open Anyway. FIRST-RUN.txt"
  note "inside the image says so, in those words."
fi
