# The tab strip: what it does

Agreed 2026-08-04. This is the behaviour specification — *what the strip is and
how it responds* — settled before any more implementation, because the previous
round of defects were not implementation errors. They were the absence of this
document: every unanswered question here had grown a branch in the code, and
those branches disagreed with each other.

Companion: `SUISEI-TAB-STRIP-GEOMETRY.md` covers *where things are drawn* and
why measuring rather than computing made it unstable. Read this one first — the
geometry only becomes tractable once the model below is fixed.

---

## 1. What the strip is a view of

An ordered list of **entries**. An entry is one of:

```
Entry = Document(bufferId)
      | Layout(layoutId, style, members[])          style ∈ { grouped, merged }
```

**The entry list does not change when a layout's style changes.** Merging does
not remove anything; one entry draws itself differently. This is the load-bearing
rule. Previously merging deleted the member chips from the list, so the list's
shape depended on presentation — which churned view identity, stranded
measurements, and forced `TabScene.id` to carry two id namespaces (a buffer id
for documents, a layout id for merged chips) in one field.

Chips are *derived* from entries, and only the chip list varies:

| entry | style | chips drawn |
|---|---|---|
| Document | — | 1, the document |
| Layout | grouped | N, one per member, with a band behind the run |
| Layout | merged | 1, carrying the layout's name |

---

## 2. Selection

**Selection is always the focused document.** There is exactly one highlighted
thing at any time.

| state | what is highlighted |
|---|---|
| a Document entry is focused | that chip |
| a grouped Layout, member focused | that member's chip |
| a merged Layout, member focused | the layout chip, **as a proxy** |

The merged case is a defined proxy, not an ambiguity: the focused document has
no chip of its own while merged, so its owning entry carries the highlight.

**`app.active_layout` is not a second selection.** It records which arrangement
is loaded, nothing more — the tab strip never highlights "a layout" as such.

But it is not inert either, and an earlier revision of this document was wrong
about that. It said `selectTabChip`'s `isLayoutDeskActive || editorSplit.isSplit`
branch "was a guess standing in for this rule", and deleting it on that basis
broke two things at once: parked layouts stopped installing their split, and
grouped ⇄ loose stopped working entirely, because `unfold_layout` is defined
against the *active* layout and returns false when there is none
(`suisei-core/src/layouts.rs`). Selection and loadedness are genuinely separate
questions; §3 answers the second.

---

## 3. Click

**Merged is opaque**: a merged layout's members are not reachable from the strip.
To reach one, un-merge to grouped.

| chip clicked | effect |
|---|---|
| Document | focus that document; leave any active layout |
| grouped member | focus that document *within* the layout — see below |
| merged layout | restore the whole arrangement |

"Within the layout" has to say what happens when that layout is not the
arrangement currently on screen, and this is where the rule earned its
condition:

| desk state | clicking a grouped member |
|---|---|
| this layout owns the desk | focus the document in place; do not reinstall the tree |
| a *free* split the user built | focus the document; do **not** clobber their arrangement |
| otherwise (parked) | install the layout, focused on that document |

The third row is what makes a folded layout's split reappear, and it is also
what sets `active_layout` — without which §7's grouped ⇄ loose step has nothing
to act on. Pinned by
`suisei-engine/tests/grouped_member_click_installs_layout.rs`.

---

## 4. Hover

Hover resolves through **the same hit test as click**. Exactly one entry is
hovered at a time, and it is always the entry a click at that position would
select.

This is a behavioural guarantee, not an implementation note. Previously clicks
went through `TabStripMouse` → `tabSlot` and hover through each chip's own
`.onHover` — two authorities, free to disagree, and they did: hovering one chip
highlighted and then selected another.

---

## 5. Close (✕)

**Closing an entry closes that entry**, and only it.

| chip | effect |
|---|---|
| Document | close that document |
| grouped member | close that document; if the layout drops below 2 documents it dissolves |
| merged layout | drop the layout; its documents remain open as ordinary Document entries |

Closing a layout never closes documents. The gesture reads the same everywhere:
it removes the thing you clicked, not the things inside it.

---

## 6. Invariants

These are the specification in testable form. Each is checkable headlessly
against the model, with no window and no app build — which is the property that
was missing while these bugs were being chased.

1. **Entry count is invariant under a style toggle.** grouped ⇄ merged changes
   the chip count, never the entry count.
2. **A hit test returns 0 or 1 entries**, never more.
3. **Exactly one entry is highlighted**, always.
4. **While merged, no member's bufferId is hit-testable.** Opacity is enforced,
   not merely undrawn.
5. **Click and hover agree.** For any x, the entry hover reports and the entry a
   click would select are the same.

Invariant 5 is the one that makes the reported bug impossible rather than fixed.
Under the previous structure it could not even be stated, because the two paths
did not share a notion of "the entry at x".

---

## 7. What this removes

Falling out of the rules above, not as separate work:

- `selectTabChip`'s dispatch on *which kind of chip this is* → one model lookup
  (its dispatch on *what the desk is currently showing* stays — that is §3's
  table, and it is not the strip guessing about anything)
- the second selection concept (`active_layout` as a highlight) → gone
- per-chip `.onHover` → hover comes from the hit test
- the two id namespaces in `TabScene.id` → entries are typed
- transitions that restructure the whole row → a style change is local to one
  entry
