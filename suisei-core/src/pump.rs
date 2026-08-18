//! Background pump for the asynchronous language services (LSP, DAP, call
//! hierarchy, hooks).
//!
//! Both clients work the same way: a request is written to a child process'
//! stdin and a reader thread pushes replies onto a channel. **Nothing happens
//! until somebody drains that channel.** For the LSP that drain is also what
//! advances the handshake — [`crate::lsp::LspClient::poll`] parses the
//! `initialize` result and only then sends `initialized` + the first
//! `textDocument/didOpen`. A frontend that never calls it leaves the server
//! spawned but un-handshaked: rust-analyzer answers `initialize` and then sits
//! idle at ~10 MB RSS forever, `server_running` stays false, and every request
//! (references, hover, definition, rename, format) resolves empty.
//!
//! The xei TUI drove all of this inline in its main loop; the GUI engine never
//! had an equivalent, which is exactly the bug this module closes. The engine
//! tick calls [`App::poll_language_services`] once per frame.
//!
//! Two deliberate differences from the TUI's version of this block:
//!
//! - **References are not drained here.** The face polls
//!   [`App::references_result`] until it reports ready, so the list has to stay
//!   put; the TUI instead consumed it into an XLC dump.
//! - **The result is a repaint hint, not a full invalidation.** LSP traffic
//!   arrives in bursts during indexing, and a full recompose per message would
//!   rebuild the outline at burst rate. Only a navigator-visible change
//!   (diagnostic count, server up/down, a palette or tab change) asks for one.

use crate::app::App;

/// What a pump call changed, so the caller can pick the cheapest repaint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PumpChange {
    /// Editor-band paint is stale: diagnostics, semantic tokens, inlay hints.
    pub paint: bool,
    /// Shell chrome is stale: diagnostic counts, tabs, palette, status message.
    pub chrome: bool,
}

impl PumpChange {
    pub fn any(self) -> bool {
        self.paint || self.chrome
    }
}

/// Cap on hover text handed to the face.
///
/// Was 800, "matching the TUI's popup budget" — a budget belonging to a face
/// that no longer exists (the workspace is core, engine and daemon; the Mac
/// app is the only face, and its card scrolls). What a language server sends
/// for a keyword is a guide, not a tooltip: rust-analyzer's answer for `fn` is
/// 1,834 characters — what a function is, where one may be written, and four
/// worked examples. At 800 it arrived cut off mid-sentence with every example
/// gone, which is the half that answers "how do I use it".
///
/// Still bounded, because a doc comment can be arbitrarily long and this
/// crosses a fixed-size ABI buffer (`SUISEI_HOVER_TEXT`). Kept comfortably
/// under it so multi-byte text cannot overflow: the truncation there is by
/// bytes, and this one is by characters.
const HOVER_CHARS: usize = 4000;

/// How long a ⌘S waits for the formatter before writing anyway.
///
/// VS Code uses 750 ms for the same trade and it is the right order: long
/// enough that a warm `rust-analyzer` makes it, short enough that a cold or
/// wedged one does not turn ⌘S into a pause the hand notices.
const FORMAT_ON_SAVE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(750);

impl App {
    /// Drain the language services and apply whatever arrived. Call once per
    /// frame; it is a cheap no-op (two empty `try_recv`s) when nothing is
    /// pending. Returns which surfaces the caller should repaint.
    pub fn poll_language_services(&mut self) -> PumpChange {
        let mut change = PumpChange::default();

        // Navigator-visible quantities, sampled before the drain.
        let diagnostics_revision_before = self.lsp.diagnostics_revision;
        let running_before = self.lsp.server_running;
        let build_revision_before = self.build.revision;

        if self.lsp.poll() {
            change.paint = true;
        }
        self.poll_call_hierarchy();
        self.poll_hook_messages();

        // The extent marks rows AWAY from the caret, and nothing else
        // repaints those. Without this the part of the bracket on the caret's
        // own line kept being redrawn — that row repaints constantly — and the
        // rest was drawn once and then never again: "라인 여러개를 차지하는
        // 브라켓은 바로 사라짐".
        if self.refresh_value_extent() {
            change.paint = true;
        }

        self.dap.poll();

        // The build, on the same tick as the debugger and for the same reason:
        // it is a process that talks, and nothing else is listening.
        let build_state_before = self.build.state;
        let build_lines_before = self.build.output.len();
        self.build.poll();
        // The FILE can change without the build saying anything, so this is
        // asked every tick rather than only when a build ends.
        self.sync_build_diagnostics();
        if self.build.state != build_state_before {
            change.chrome = true;
            change.paint = true;
        }
        if self.build.output.len() != build_lines_before {
            change.chrome = true;
        }

        if self.dap.location_dirty {
            self.dap_apply_stopped_location();
            change.chrome = true;
            change.paint = true;
        }

        // Server-side completions merge into the list the core already built,
        // but only while it is on screen — otherwise they are stale by arrival.
        let lsp_comps = std::mem::take(&mut self.lsp.pending_completions);
        if !lsp_comps.is_empty() && self.completions.active {
            for item in lsp_comps {
                if self
                    .completions
                    .suggestions
                    .iter()
                    .any(|s| s.label == item.label)
                {
                    continue;
                }
                self.completions
                    .suggestions
                    .push(crate::completion::Suggestion {
                        label: item.label.clone(),
                        detail: item.detail.unwrap_or_else(|| "LSP".to_string()),
                        insert_text: item.label,
                    });
            }
            change.chrome = true;
        }

        if let Some(loc) = self.lsp.pending_definition.take() {
            self.apply_definition(loc);
            change.chrome = true;
            change.paint = true;
        }

        // Document / workspace symbols → palette.
        if !self.lsp.pending_symbols.is_empty() {
            self.apply_pending_symbols();
            change.chrome = true;
        }

        // Inlay hints and code lenses re-request themselves; both calls are
        // gated on their own dirty flag, so this is free on a quiet frame.
        if let Some(path) = self.filename.clone() {
            let path_s = path.display().to_string();
            if self.inlay_hints_enabled {
                let end = self.buffer.line_count().saturating_sub(1);
                self.lsp.maybe_request_inlays(&path_s, end);
            }
            if self.code_lens_enabled {
                self.lsp.maybe_request_code_lens(&path_s);
            }
        }

        if let Some(hover) = self.lsp.pending_hover.take() {
            self.hover_text = Some(hover.chars().take(HOVER_CHARS).collect());
            change.chrome = true;
        }

        // Multi-file full-text edits (rename / format / code action apply).
        if !self.lsp.pending_edits.is_empty() {
            let edits = std::mem::take(&mut self.lsp.pending_edits);
            // A save held for the formatter: this reply describes the document
            // as it was when we asked, and applying it replaces the WHOLE
            // buffer. If the user kept typing while the server thought, that
            // replacement would put the file back and take the keystrokes with
            // it. A version that has moved means the answer is about a document
            // that no longer exists.
            let held = self
                .pending_save
                .as_ref()
                .filter(|_| self.lsp.formatting_answered);
            let stale = held.is_some_and(|p| p.version != self.buffer.version());
            if stale {
                self.set_message("Kept typing while formatting — saved as typed");
            } else {
                self.apply_file_edits(edits);
            }
            change.chrome = true;
            change.paint = true;
        }

        // The formatter answered. Edits or not — "nothing to change" is an
        // answer too — the save it was holding can go through now.
        if self.lsp.formatting_answered {
            self.lsp.formatting_answered = false;
            if self.pending_save.take().is_some() {
                self.save_file();
                change.chrome = true;
                change.paint = true;
            }
        }

        // …and if it never answers, the file is still written. A save must not
        // be lost to a hung language server; unformatted beats unsaved.
        if let Some(p) = &self.pending_save {
            if p.asked_at.elapsed() >= FORMAT_ON_SAVE_TIMEOUT {
                self.pending_save = None;
                self.save_file();
                self.set_message("Formatter did not answer — saved unformatted");
                change.chrome = true;
                change.paint = true;
            }
        }
        if let Some(msg) = self.lsp.pending_workspace_edit.take() {
            // "APPLY\n…" is the payload form handled by apply_file_edits above;
            // anything else is a status note.
            if !msg.starts_with("APPLY\n") {
                self.message = msg;
                change.chrome = true;
            }
        }

        if !self.lsp.pending_code_actions.is_empty() {
            self.open_code_actions_palette();
            change.chrome = true;
        }

        // A soft error is advisory (missing binary, unsupported request); show
        // it once rather than letting it accumulate unread.
        if let Some(soft) = self.lsp.soft_error.take() {
            if self.message.is_empty() || self.message.ends_with('…') {
                self.message = soft;
                change.chrome = true;
            }
        }

        if self.lsp.diagnostics_revision != diagnostics_revision_before
            || self.lsp.server_running != running_before
            || self.build.revision != build_revision_before
        {
            change.chrome = true;
            change.paint = true;
        }
        change
    }

    /// Land a `textDocument/definition` answer: peek popover or a real jump.
    /// The server reports UTF-16 columns, so the target line has to be read
    /// (from the buffer when it is the current file) to convert.
    fn apply_definition(&mut self, loc: crate::lsp::Location) {
        let as_peek = self.lsp.definition_as_peek;
        self.lsp.definition_as_peek = false;

        if as_peek {
            let line = std::fs::read_to_string(&loc.path)
                .ok()
                .and_then(|t| t.lines().nth(loc.row).map(|l| l.to_string()))
                .unwrap_or_default();
            let col = crate::lsp::utf16_to_char_col(&line, loc.col);
            self.open_peek_at(&loc.path, loc.row, col);
            return;
        }

        let same_file = self
            .filename
            .as_ref()
            .map(|p| p.display().to_string() == loc.path)
            .unwrap_or(false);
        if !same_file {
            self.open_new_tab(&loc.path);
        }
        let row = loc.row.min(self.buffer.line_count().saturating_sub(1));
        let col = crate::lsp::utf16_to_char_col(self.buffer.line(row), loc.col);
        self.buffer.cursor.row = row;
        self.buffer.cursor.col = col;
        self.buffer.clamp_col();
        // The GUI's selection set is the caret authority — leave a single
        // collapsed caret at the target, not a stale multi-cursor set.
        self.sel = crate::selection::SelectionSet::single(crate::selection::Selection::caret(
            self.buffer.cursor(),
        ));
        self.update_scroll();
        self.message = format!("Jumped to definition: {}:{}", loc.path, loc.row + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::Location;

    #[test]
    fn idle_pump_reports_no_change() {
        let mut app = App::new();
        assert!(
            !app.poll_language_services().any(),
            "a quiet frame must not ask for a repaint"
        );
    }

    /// Unlike the TUI's version of this block, the pump must NOT consume the
    /// references list: the Find navigator polls `references_result()` for as
    /// long as the panel is open, so draining it here would empty the panel one
    /// frame after it filled.
    #[test]
    fn references_survive_the_pump() {
        let mut app = App::new();
        app.lsp.pending_references = vec![Location {
            path: "/tmp/a.rs".into(),
            row: 3,
            col: 7,
        }];
        app.lsp.references_ready = true;
        app.poll_language_services();
        let (refs, ready) = app.references_result();
        assert!(ready);
        assert_eq!(refs.len(), 1, "the face still needs this list");
    }

    #[test]
    fn hover_answer_lands_in_hover_text_and_asks_for_a_repaint() {
        let mut app = App::new();
        app.lsp.pending_hover = Some("fn main()".into());
        let change = app.poll_language_services();
        assert_eq!(app.hover_text.as_deref(), Some("fn main()"));
        assert!(change.chrome);
    }

    /// Hover text is size-bounded; a 4k-line doc comment must not be handed to
    /// the face whole. Bounded, not short: rust-analyzer's guide for the `fn`
    /// keyword is 1,834 characters and has to arrive with its examples.
    #[test]
    fn hover_text_is_capped() {
        let mut app = App::new();
        app.lsp.pending_hover = Some("x".repeat(HOVER_CHARS * 2));
        app.poll_language_services();
        assert_eq!(app.hover_text.as_deref().map(str::len), Some(HOVER_CHARS));
    }
}
