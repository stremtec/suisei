//! Shell-neutral key dispatch — shared by xei TUI and Suisei.
//!
//! Extracted from `xei/src/event.rs` so both faces call `App::dispatch`.

use crate::app::{App, Mode};
use crate::key::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    /// Single key-entry for all shells. Behavior must match the TUI path.
    pub fn dispatch(&mut self, ev: KeyEvent) {
        dispatch_key(self, ev.code, ev.modifiers);
    }
}

/// Paste OS clipboard into the terminal PTY that owns the keyboard — the
/// focused pane's shell when there is one, else the dock (text, or image
/// path). The old dock-only target silently dropped ⌘V in pane terminals.
fn paste_clipboard_to_terminal(app: &mut App) {
    if let Some(text) = crate::clipboard::paste() {
        if !text.is_empty() {
            if let Some(t) = app.focused_pane_terminal_mut() {
                t.paste_input(&text);
            } else {
                app.terminal.paste_input(&text);
            }
            return;
        }
    }
    if let Some(p) = crate::clipboard::paste_image_to_temp() {
        let path = p.to_string_lossy().to_string();
        if let Some(t) = app.focused_pane_terminal_mut() {
            t.paste_input(&path);
        } else {
            app.terminal.paste_input(&path);
        }
        app.message = String::from("Pasted image → terminal");
    }
}

fn dispatch_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    // ── Pane terminal (Ctrl+Shift+T): strict PTY policy ─────────────────
    // When the terminal *window* is focused, almost every key goes to the
    // child shell (Ctrl+C, arrows, …). Only a tiny allowlist is editor chrome.
    // Must run before clipboard / Ctrl+S / Cmd+C handlers.
    if app.terminal_window_focused() && matches!(app.mode, Mode::Editor) {
        if handle_pane_terminal_window(app, code, modifiers) {
            return;
        }
        // false → allowlisted editor action already handled (e.g. split chord
        // continues below for Ctrl+W second key). Fall through only for that.
    }

    // Cmd (macOS) or Ctrl+Shift (common terminal) clipboard shortcuts
    let cmd_like = modifiers.contains(KeyModifiers::SUPER)
        || (modifiers.contains(KeyModifiers::CONTROL) && modifiers.contains(KeyModifiers::SHIFT));
    if cmd_like {
        match code {
            // macOS-standard text motions — the GUI face routes ⌘-arrows and
            // ⌘⌫ straight here; without these they fell into vim handling
            // ("shortcuts feel vim-tuned" complaint).
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Backspace
                if matches!(app.mode, Mode::Editor) =>
            {
                match code {
                    KeyCode::Left => app.buffer.move_to_line_start(),
                    KeyCode::Right => app.buffer.move_to_line_end(),
                    KeyCode::Up => app.goto_line(1),
                    KeyCode::Down => {
                        let last = app.buffer.line_count().max(1);
                        app.goto_line(last);
                    }
                    KeyCode::Backspace => {
                        // ⌘⌫ — delete to line start.
                        let n = app.buffer.cursor.col;
                        for _ in 0..n {
                            app.buffer.backspace();
                        }
                        app.modified = true;
                    }
                    _ => {}
                }
                app.update_scroll();
                return;
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                // cmd_like+C → copy. Shift handled upstream as paste-preview; C is plain copy.
                if !matches!(
                    app.mode,
                    Mode::Terminal | Mode::Palette | Mode::SourceControl
                ) {
                    app.clipboard_copy();
                }
                return;
            }
            KeyCode::Char('v') | KeyCode::Char('V') => {
                // Terminal focused → paste into the child PTY (text or image path),
                // not the editor. (Pane-window terminal is handled earlier.)
                if app.terminal_window_focused()
                    || (app.terminal.open && app.mode == Mode::Terminal)
                {
                    paste_clipboard_to_terminal(app);
                    return;
                }
                // Shift+V under cmd_like → pretty preview toggle (VS Code Markdown preview).
                if modifiers.contains(KeyModifiers::SHIFT) {
                    if matches!(app.mode, Mode::Editor | Mode::Preview | Mode::Explorer) {
                        app.toggle_preview();
                    }
                    return;
                }
                // plain cmd_like+V → paste
                if !matches!(
                    app.mode,
                    Mode::Terminal | Mode::Palette | Mode::SourceControl | Mode::Preview
                ) {
                    app.clipboard_paste();
                }
                return;
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                // Cmd/Ctrl+X — cut the selection.
                if matches!(app.mode, Mode::Editor) && app.has_selection() {
                    app.clipboard_copy();
                    app.delete_selection();
                    app.message = String::from("Cut to clipboard");
                }
                return;
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                // Cmd/Ctrl+Shift+P — command palette (VS Code)
                if modifiers.contains(KeyModifiers::SHIFT) {
                    app.open_command_palette();
                    return;
                }
                // Cmd+P alone → file palette
                if modifiers.contains(KeyModifiers::SUPER) {
                    app.open_file_palette();
                    return;
                }
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                // Ctrl+Shift+F — find in files
                if modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(
                        app.mode,
                        Mode::Editor | Mode::WorkspaceSearch | Mode::Explorer
                    )
                {
                    app.open_workspace_search();
                    return;
                }
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                // Ctrl+Shift+T — full-panel terminal (editor slot)
                if modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(
                        app.mode,
                        Mode::Editor | Mode::Terminal | Mode::Explorer | Mode::GitWorkbench
                    )
                {
                    app.toggle_terminal_full();
                    return;
                }
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                // Ctrl+Shift+O — document symbols
                if modifiers.contains(KeyModifiers::SHIFT)
                    && matches!(app.mode, Mode::Editor | Mode::Explorer)
                {
                    app.open_document_symbols();
                    return;
                }
            }
            KeyCode::Char('i') | KeyCode::Char('I') => {
                // Ctrl+Shift+I — format document (VS Code-ish)
                if modifiers.contains(KeyModifiers::SHIFT) && matches!(app.mode, Mode::Editor) {
                    app.format_document();
                    return;
                }
            }

            KeyCode::Char('g') | KeyCode::Char('G') => {
                // Ctrl+Shift+G — full Git workbench
                // Ctrl+G — light Source Control
                let git_modes = matches!(
                    app.mode,
                    Mode::Editor | Mode::SourceControl | Mode::GitWorkbench | Mode::Explorer
                );
                if !git_modes {
                    return;
                }
                if modifiers.contains(KeyModifiers::SHIFT) {
                    app.toggle_git_workbench();
                } else {
                    app.toggle_scm();
                }
                return;
            }
            KeyCode::Char(',') => {
                // Ctrl/Cmd+, — Settings (VS Code convention)
                if matches!(
                    app.mode,
                    Mode::Editor | Mode::Settings | Mode::Explorer | Mode::GitWorkbench
                ) {
                    app.open_settings();
                }
                return;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                // Cmd+S (macOS) — save (Ctrl+S is handled in the CONTROL block below)
                if matches!(app.mode, Mode::Editor) {
                    app.save_file();
                }
                return;
            }
            _ => {}
        }
    }

    // Editor right-click menu keyboard navigation
    if app.editor_ctx.is_some() {
        match code {
            KeyCode::Esc => {
                app.close_editor_ctx();
                app.message = String::new();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(m) = app.editor_ctx.as_mut() {
                    m.sel = m.sel.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(m) = app.editor_ctx.as_mut() {
                    let max = m.items.len().saturating_sub(1);
                    m.sel = (m.sel + 1).min(max);
                }
            }
            KeyCode::Enter => match app.run_editor_ctx_action() {
                Ok(msg) => app.message = msg,
                Err(e) => app.message = e,
            },
            _ => {}
        }
        return;
    }

    // Peek overlay captures keys while open (before mode dispatch).
    if app.peek.open {
        match code {
            KeyCode::Esc => {
                app.peek.close();
                app.message = String::new();
                return;
            }
            KeyCode::Enter => {
                app.promote_peek();
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.peek.scroll_by(1);
                return;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.peek.scroll_by(-1);
                return;
            }
            _ => {}
        }
    }

    // Ctrl+W split chords. Work even when a terminal *window*
    // is focused — pane terminal is not Mode::Terminal.
    if app.split.pending_chord
        && matches!(app.mode, Mode::Editor)
        && !modifiers.contains(KeyModifiers::CONTROL)
    {
        app.split.pending_chord = false;
        match code {
            KeyCode::Char('v') => app.split_vertical(),
            KeyCode::Char('s') => app.split_horizontal(),
            KeyCode::Char('w') | KeyCode::Char('W') => app.focus_other_pane(),
            KeyCode::Char('q') => app.close_split(),
            KeyCode::Char('=') => {
                app.split.equalize();
                app.message = String::from("Split equalized");
            }
            KeyCode::Char('h') | KeyCode::Left => app.focus_dir('h'),
            KeyCode::Char('l') | KeyCode::Right => app.focus_dir('l'),
            KeyCode::Char('k') | KeyCode::Up => app.focus_dir('k'),
            KeyCode::Char('j') | KeyCode::Down => app.focus_dir('j'),
            KeyCode::Char('>') => app.split.adjust_focused(0.05),
            KeyCode::Char('<') => app.split.adjust_focused(-0.05),
            KeyCode::Esc => {
                app.message.clear();
            }
            _ => {
                app.message = String::from("Ctrl+W: v/s split · w cycle · q close · = equal");
            }
        }
        return;
    }

    // ⌥ word motions / ⌥⌫ word delete — macOS text-editing standard
    // (GUI face routes Option-arrows here).
    if modifiers.contains(KeyModifiers::ALT)
        && !cmd_like
        && !modifiers.contains(KeyModifiers::CONTROL)
        && matches!(app.mode, Mode::Editor)
    {
        match code {
            KeyCode::Left => {
                app.buffer.move_word_back();
                app.update_scroll();
                return;
            }
            KeyCode::Right => {
                app.buffer.move_word_forward();
                app.update_scroll();
                return;
            }
            KeyCode::Backspace => {
                // Delete back one word: run of spaces, then word chars.
                while app.buffer.cursor.col > 0
                    && matches!(app.buffer.char_before_cursor(), Some(c) if c.is_whitespace())
                {
                    app.buffer.backspace();
                }
                while app.buffer.cursor.col > 0
                    && matches!(
                        app.buffer.char_before_cursor(),
                        Some(c) if c.is_alphanumeric() || c == '_'
                    )
                {
                    app.buffer.backspace();
                }
                app.modified = true;
                app.update_scroll();
                return;
            }
            _ => {}
        }
    }

    // Ctrl+V → paste.
    if modifiers.contains(KeyModifiers::CONTROL)
        && !modifiers.contains(KeyModifiers::SHIFT)
        && !modifiers.contains(KeyModifiers::SUPER)
    {
        if matches!(code, KeyCode::Char('w') | KeyCode::Char('W')) && app.mode == Mode::Editor {
            app.split.pending_chord = true;
            app.message = String::from("Ctrl+W — s split · v vsplit · w focus · q close");
            return;
        }
        if matches!(code, KeyCode::Char('.')) && matches!(app.mode, Mode::Editor) {
            // Ctrl+. — code actions / quick fix
            app.request_code_actions();
            return;
        }
        if matches!(code, KeyCode::Char('v') | KeyCode::Char('V')) && app.mode == Mode::Editor {
            app.clipboard_paste();
            return;
        }
    }

    // ── DAP debug function keys (any editor mode) ───────────────────
    if matches!(app.mode, Mode::Editor | Mode::Debug) {
        match code {
            KeyCode::F(5) => {
                if modifiers.contains(KeyModifiers::SHIFT) {
                    app.dap_stop();
                } else {
                    app.dap_start_or_continue();
                }
                return;
            }
            KeyCode::F(6) => {
                app.dap_pause();
                return;
            }
            KeyCode::F(9) => {
                app.dap_toggle_breakpoint();
                return;
            }
            KeyCode::F(10) => {
                app.dap_step_over();
                return;
            }
            KeyCode::F(11) => {
                if modifiers.contains(KeyModifiers::SHIFT) {
                    app.dap_step_out();
                } else {
                    app.dap_step_into();
                }
                return;
            }
            _ => {}
        }
    }

    if code == KeyCode::F(12) {
        if app.terminal.open {
            app.terminal.open = false;
            app.terminal.shutdown();
            app.mode = Mode::Editor;
        } else {
            app.terminal.open = true;
            app.terminal.start(app.filename.as_ref());
            app.mode = Mode::Terminal;
        }
        return;
    }

    if modifiers.contains(KeyModifiers::CONTROL) {
        let ctrl_char = match code {
            KeyCode::Char(c) => c,
            _ => {
                /* fall through */
                '?'
            }
        };

        if ctrl_char == 'q' {
            if app.terminal.open {
                app.terminal.open = false;
                app.terminal.shutdown();
                app.mode = Mode::Editor;
            }
            return;
        }

        if ctrl_char == 't' {
            // Ctrl+T alone (no Shift) — side panel terminal
            if !modifiers.contains(KeyModifiers::SHIFT) {
                app.toggle_terminal_side();
                return;
            }
        }

        if app.mode != Mode::Terminal {
            // Multi-cursor: Ctrl+D add next match · Ctrl+Alt+j/k column carets
            if matches!(code, KeyCode::Char('d') | KeyCode::Char('D'))
                && !modifiers.contains(KeyModifiers::SHIFT)
                && matches!(app.mode, Mode::Editor)
            {
                app.multi_cursor_add_next();
                return;
            }
            if modifiers.contains(KeyModifiers::ALT) && matches!(app.mode, Mode::Editor) {
                match code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.multi_cursor_add_below();
                        return;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.multi_cursor_add_above();
                        return;
                    }
                    _ => {}
                }
            }
            match code {
                KeyCode::Char('s') => {
                    // Ctrl+S save
                    if matches!(app.mode, Mode::Editor) {
                        app.save_file();
                    }
                    return;
                }
                KeyCode::Char('r') => {
                    // Ctrl+R redo
                    if app.mode == Mode::Editor {
                        app.redo();
                    }
                    return;
                }
                KeyCode::Char('o') => {
                    // Ctrl+O jump back
                    if app.mode == Mode::Editor {
                        app.jump_back();
                    }
                    return;
                }
                KeyCode::Char('i') => {
                    // Ctrl+I jump forward
                    if app.mode == Mode::Editor {
                        app.jump_forward();
                    }
                    return;
                }
                KeyCode::Char('p') => {
                    // Ctrl+P — quick open files (VS Code)
                    if app.mode == Mode::Editor || app.mode == Mode::Editor {
                        app.open_file_palette();
                    }
                    return;
                }
                KeyCode::Char('a') => {
                    if app.mode == Mode::Editor {
                        trigger_completion(app);
                    }
                    return;
                }
                KeyCode::Char('g') => {
                    // Ctrl+G — light SCM (from workbench: step back to SCM)
                    if matches!(
                        app.mode,
                        Mode::Editor | Mode::SourceControl | Mode::GitWorkbench | Mode::Explorer
                    ) {
                        app.toggle_scm();
                    }
                    return;
                }
                KeyCode::Char('d') | KeyCode::Char('D')
                    if modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    // Ctrl+Shift+D — debug panel (VS Code-ish)
                    if matches!(app.mode, Mode::Editor | Mode::Debug | Mode::Explorer) {
                        app.toggle_debug_panel();
                    }
                    return;
                }
                KeyCode::Char(',') => {
                    // Ctrl+, without requiring Shift (some terminals only send CONTROL)
                    if matches!(
                        app.mode,
                        Mode::Editor | Mode::Settings | Mode::Explorer | Mode::GitWorkbench
                    ) {
                        app.open_settings();
                    }
                    return;
                }
                KeyCode::Char('b') | KeyCode::Char('B') => {
                    // Ctrl+B — git blame side panel (slide-in, flame colors)
                    if matches!(app.mode, Mode::Editor | Mode::Explorer) {
                        app.toggle_blame();
                        return;
                    }
                }
                KeyCode::Char('f') => {
                    if matches!(app.mode, Mode::Editor | Mode::Explorer) {
                        if app.explorer.open {
                            app.explorer.close();
                            app.mode = Mode::Editor;
                        } else {
                            app.explorer.toggle_at(app.filename.as_ref());
                            app.mode = Mode::Explorer;
                        }
                    }
                    return;
                }
                _ => {}
            }
        } else if let KeyCode::Char(c) = code {
            let ctrl_byte = if c.is_ascii_lowercase() {
                c as u8 - b'a' + 1
            } else {
                c as u8
            };
            app.terminal.write_input(&[ctrl_byte]);
            return;
        }
    }

    // Route to whichever surface owns the keyboard. The editor is absent on
    // purpose: `Mode::Editor` keys are handled by the engine's Selection-model
    // tables and never arrive here (see `Engine::dispatch_key`). Anything that
    // does reach this arm is an application chord the sections above declined.
    match app.mode {
        Mode::Editor => {}
        Mode::Explorer => handle_explorer(app, code),
        Mode::Terminal => handle_terminal(app, code, modifiers),
        Mode::Search => handle_search_input(app, code),
        Mode::Palette => handle_palette(app, code),
        Mode::SourceControl => handle_scm(app, code),
        Mode::GitWorkbench => handle_git_workbench(app, code),
        Mode::Settings => handle_settings(app, code),
        Mode::Preview => handle_preview(app, code),
        Mode::WorkspaceSearch => handle_workspace_search(app, code),
        Mode::Debug => handle_debug(app, code),
        Mode::CallHierarchy => handle_call_hierarchy(app, code),
    }
}

fn handle_palette(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.palette.close();
            app.mode = Mode::Editor;
            app.message = String::new();
        }
        KeyCode::Enter => app.execute_palette_selection(),
        // `j`/`k` used to move the selection while the query was empty — vim
        // habit that made those two letters untypeable as the first character
        // of a filter. Arrows move; letters always type.
        KeyCode::Down => app.palette.move_down(),
        KeyCode::Up => app.palette.move_up(),
        KeyCode::Backspace => app.palette.pop_char(),
        KeyCode::Char(c) if !c.is_control() => app.palette.push_char(c),
        _ => {}
    }
}

fn handle_workspace_search(app: &mut App, code: KeyCode) {
    use crate::workspace_search::replace_in_file;

    match code {
        KeyCode::Esc => {
            app.workspace_search.close();
            app.mode = Mode::Editor;
            app.message = String::new();
        }
        KeyCode::Tab => {
            app.workspace_search.toggle_replace_focus();
        }
        KeyCode::Backspace => {
            app.workspace_search.pop_char();
            if !app.workspace_search.replace_focus {
                app.workspace_search.run_search();
            }
        }
        KeyCode::Down | KeyCode::Char('j')
            if app.workspace_search.replace_focus || app.workspace_search.query.is_empty() =>
        {
            app.workspace_search.move_sel(1);
        }
        KeyCode::Down => app.workspace_search.move_sel(1),
        KeyCode::Up | KeyCode::Char('k') => app.workspace_search.move_sel(-1),
        KeyCode::Enter => {
            if app.workspace_search.needs_search || app.workspace_search.hits.is_empty() {
                app.workspace_search.run_search();
            }
            if let Some(hit) = app.workspace_search.selected_hit().cloned() {
                app.workspace_search.close();
                app.mode = Mode::Editor;
                app.goto_file_location(&hit.path.display().to_string(), hit.row, hit.col);
            }
        }
        KeyCode::Char('r') if !app.workspace_search.replace_focus => {
            // replace one at selection
            let q = app.workspace_search.query.clone();
            let repl = app.workspace_search.replace.clone();
            if q.is_empty() {
                app.workspace_search.status = "Nothing to replace".into();
                return;
            }
            if let Some(hit) = app.workspace_search.selected_hit().cloned() {
                match replace_in_file(&hit.path, hit.row, &q, &repl) {
                    Ok(true) => {
                        // reload if open
                        if app.filename.as_ref() == Some(&hit.path) {
                            if let Ok(content) = std::fs::read_to_string(&hit.path) {
                                app.push_undo();
                                app.buffer = crate::buffer::Buffer::from_string(&content);
                                app.modified = false;
                            }
                        }
                        app.workspace_search.run_search();
                        app.workspace_search.status = format!("Replaced in {}", hit.path.display());
                    }
                    Ok(false) => {
                        app.workspace_search.status = "Pattern not found on line".into();
                    }
                    Err(e) => {
                        app.workspace_search.status = e;
                    }
                }
            }
        }
        KeyCode::Char('R') if !app.workspace_search.replace_focus => {
            let q = app.workspace_search.query.clone();
            let repl = app.workspace_search.replace.clone();
            if q.is_empty() {
                return;
            }
            let hits = app.workspace_search.hits.clone();
            let mut n = 0usize;
            for hit in &hits {
                if replace_in_file(&hit.path, hit.row, &q, &repl).unwrap_or(false) {
                    n += 1;
                }
            }
            app.workspace_search.run_search();
            app.workspace_search.status = format!("Replaced {n} occurrence(s)");
        }
        KeyCode::Char(c) if !c.is_control() => {
            app.workspace_search.push_char(c);
            if !app.workspace_search.replace_focus {
                // live search for short queries; debounce by always running (rg is fast)
                if app.workspace_search.query.len() >= 2 {
                    app.workspace_search.run_search();
                }
            }
        }
        _ => {}
    }
}

fn handle_settings(app: &mut App, code: KeyCode) {
    use crate::settings::{SettingsAction, SettingsPage};

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.close_settings();
        }
        KeyCode::Tab => app.settings.next_page(),
        KeyCode::BackTab => app.settings.prev_page(),
        KeyCode::Down | KeyCode::Char('j') => app.settings.move_sel(1),
        KeyCode::Up | KeyCode::Char('k') => app.settings.move_sel(-1),
        KeyCode::PageDown => {
            for _ in 0..8 {
                app.settings.move_sel(1);
            }
        }
        KeyCode::PageUp => {
            for _ in 0..8 {
                app.settings.move_sel(-1);
            }
        }
        KeyCode::Enter | KeyCode::Char(' ') => match app.settings.activate() {
            SettingsAction::ApplyTheme => app.apply_settings_draft(),
            SettingsAction::ApplyGpuAcc => {
                app.apply_settings_draft();
                // Never sticky-undercurl the whole session (paints waves on empty cells).
                // TUI-only underline SGR reset (xei/src/gpu_frame.rs)
                // no-op in headless/core dispatch
                app.message = if app.gpu_acc {
                    if app.gpu_active() {
                        "gpu_acc on".to_string()
                    } else {
                        "gpu_acc on · host is basic (limited enhancements)".into()
                    }
                } else {
                    "gpu_acc off — plain cell TUI".into()
                };
            }
            SettingsAction::ApplyLsp => {
                app.apply_settings_draft();
                app.message = app
                    .settings
                    .status
                    .clone()
                    .unwrap_or_else(|| "LSP settings applied".into());
            }
            SettingsAction::OpenWorkbench => {
                app.close_settings();
                app.open_git_workbench();
            }
            SettingsAction::OpenScm => {
                app.close_settings();
                app.toggle_scm();
            }
            SettingsAction::None => {}
        },
        KeyCode::Char('s') | KeyCode::Char('S') => {
            app.save_settings();
        }
        KeyCode::Char('1') => {
            app.settings.page = SettingsPage::About;
            app.settings.selected = 0;
        }
        KeyCode::Char('2') => {
            app.settings.page = SettingsPage::Setting;
            app.settings.selected = 1; // first theme
        }
        KeyCode::Char('3') => {
            app.settings.page = SettingsPage::Extensions;
            app.settings.selected = 0;
        }
        KeyCode::Char('4') => {
            app.settings.page = SettingsPage::Help;
            app.settings.selected = crate::settings::help_entries()
                .iter()
                .position(|e| !e.is_header)
                .unwrap_or(0);
        }
        _ => {}
    }
}

fn handle_call_hierarchy(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.call_hierarchy.close();
            app.mode = Mode::Editor;
            app.message = String::new();
        }
        KeyCode::Down | KeyCode::Char('j') => app.call_hierarchy.move_sel(1),
        KeyCode::Up | KeyCode::Char('k') => app.call_hierarchy.move_sel(-1),
        KeyCode::Tab | KeyCode::Char('t') => {
            app.toggle_call_direction();
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('o') => {
            if let Some(item) = app.call_hierarchy.selected_item().cloned() {
                if !item.path.is_empty() && std::path::Path::new(&item.path).is_file() {
                    app.push_jump();
                    app.open_new_tab(&item.path);
                    app.buffer.cursor.row = item.row.min(app.buffer.line_count().saturating_sub(1));
                    app.buffer.cursor.col = item.col;
                    app.buffer.clamp_col();
                    app.update_scroll();
                    app.call_hierarchy.close();
                    app.mode = Mode::Editor;
                    app.message = format!("→ {} · {}:{}", item.name, item.path, item.row + 1);
                }
            }
        }
        KeyCode::Char('i') => {
            // force incoming
            app.call_hierarchy.direction = crate::call_hierarchy::CallDirection::Outgoing;
            app.toggle_call_direction();
        }
        KeyCode::Char('O') => {
            app.call_hierarchy.direction = crate::call_hierarchy::CallDirection::Incoming;
            app.toggle_call_direction();
        }
        _ => {}
    }
}

/// DAP debugger panel (Ctrl+Shift+D / F5).
/// Esc unfocuses (panel stays); `q` closes the panel.
fn handle_debug(app: &mut App, code: KeyCode) {
    use crate::dap::DebugPane;

    match code {
        KeyCode::Esc => {
            // Keep panel visible — just return focus to the editor.
            app.mode = Mode::Editor;
            app.message = "Debug unfocused · Ctrl+Shift+D refocus · q closes".into();
        }
        KeyCode::Char('q') => {
            app.close_debug_panel();
        }
        KeyCode::Tab => {
            app.dap.set_pane(app.dap.pane.next());
            app.message = format!("Debug · {}", app.dap.pane.label());
        }
        KeyCode::BackTab => {
            app.dap.set_pane(app.dap.pane.prev());
        }
        KeyCode::Down | KeyCode::Char('j') => app.dap.move_focus(1),
        KeyCode::Up | KeyCode::Char('k') => app.dap.move_focus(-1),
        KeyCode::Enter | KeyCode::Char('l') => match app.dap.pane {
            DebugPane::Stack => {
                let i = app.dap.focus_row;
                app.dap.select_frame(i);
                app.dap.location_dirty = true;
                app.dap_apply_stopped_location();
            }
            DebugPane::Variables => {
                let i = app.dap.focus_row;
                app.dap.toggle_var_at(i);
            }
            // Enter also handled below for console
            DebugPane::Breakpoints => {
                let bps = app.dap.flat_bps();
                if let Some((path, line, _)) = bps.get(app.dap.focus_row) {
                    let path = path.clone();
                    let line = *line;
                    if std::path::Path::new(&path).is_file() {
                        app.open_new_tab(&path);
                        app.buffer.cursor.row = line.min(app.buffer.line_count().saturating_sub(1));
                        app.buffer.move_to_line_start();
                        app.update_scroll();
                    }
                }
            }
            DebugPane::Console => {
                let expr = app.dap.eval_input.clone();
                if !expr.is_empty() {
                    app.dap_evaluate(&expr);
                }
            }
        },
        // Collapse with `h` (tree navigation)
        KeyCode::Char('h') if app.dap.pane == DebugPane::Variables => {
            let i = app.dap.focus_row;
            if let Some(n) = app.dap.vars.get(i) {
                if n.expanded {
                    app.dap.toggle_var_at(i);
                }
            }
        }
        KeyCode::Char('c') if app.dap.pane != DebugPane::Console => app.dap_start_or_continue(),
        KeyCode::Char('n') if app.dap.pane != DebugPane::Console => app.dap_step_over(),
        KeyCode::Char('i') | KeyCode::Char('s') if app.dap.pane != DebugPane::Console => {
            app.dap_step_into()
        }
        KeyCode::Char('o') if app.dap.pane != DebugPane::Console => app.dap_step_out(),
        KeyCode::Char('p') if app.dap.pane != DebugPane::Console => app.dap_pause(),
        KeyCode::Char('r') if app.dap.pane != DebugPane::Console => {
            if let Err(e) = app.dap.restart() {
                app.message = e;
            } else {
                app.message = "▶ restart".into();
            }
        }
        KeyCode::Char('x') | KeyCode::Char('S') if app.dap.pane != DebugPane::Console => {
            app.dap_stop()
        }
        KeyCode::Char('b') if app.dap.pane != DebugPane::Console => app.dap_toggle_breakpoint(),
        KeyCode::Char('1') => app.dap.set_pane(DebugPane::Stack),
        KeyCode::Char('2') => app.dap.set_pane(DebugPane::Variables),
        KeyCode::Char('3') => app.dap.set_pane(DebugPane::Breakpoints),
        KeyCode::Char('4') => app.dap.set_pane(DebugPane::Console),
        // Console REPL typing
        KeyCode::Char(c) if app.dap.pane == DebugPane::Console && !c.is_control() => {
            app.dap.eval_input.push(c);
        }
        KeyCode::Backspace if app.dap.pane == DebugPane::Console => {
            app.dap.eval_input.pop();
        }
        _ => {}
    }
}

/// Full Git workbench (Ctrl+Shift+G) — mini GitHub surface
fn handle_git_workbench(app: &mut App, code: KeyCode) {
    use crate::git_workbench::{GitFocus, GitTab, InputMode};

    // Inline input (new branch / confirm discard) takes over typing.
    if app.git_wb.input_mode.is_some() {
        match code {
            KeyCode::Esc => {
                app.git_wb.cancel_input();
                app.message = app.git_wb.message.clone().unwrap_or_default();
            }
            KeyCode::Enter => match app.git_wb.submit_input() {
                Ok(()) => {
                    app.message = app.git_wb.message.clone().unwrap_or_default();
                    app.refresh_git();
                }
                Err(e) => app.message = e,
            },
            KeyCode::Backspace => {
                app.git_wb.input_buf.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                if matches!(app.git_wb.input_mode, Some(InputMode::NewBranch)) {
                    app.git_wb.input_buf.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    // Commit context menu (right-click on Log)
    if app.git_wb.ctx_menu.is_some() {
        match code {
            KeyCode::Esc => {
                app.git_wb.close_ctx_menu();
                app.message = String::new();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(m) = app.git_wb.ctx_menu.as_mut() {
                    m.sel = m.sel.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(m) = app.git_wb.ctx_menu.as_mut() {
                    let max = m.items.len().saturating_sub(1);
                    m.sel = (m.sel + 1).min(max);
                }
            }
            KeyCode::Enter => match app.git_wb.run_ctx_action() {
                Ok(msg) => {
                    if let Some(h) = msg.strip_prefix("Copied ") {
                        let _ = crate::clipboard::copy(h);
                    }
                    app.message = msg;
                }
                Err(e) => app.message = e,
            },
            KeyCode::Char(c) => {
                let pick = app.git_wb.ctx_menu.as_ref().and_then(|m| {
                    m.items.iter().position(|it| match c {
                        's' | 'S' => matches!(it, crate::GitCtxItem::ShowFiles),
                        'c' | 'C' => matches!(it, crate::GitCtxItem::CherryPick),
                        'v' | 'V' => matches!(it, crate::GitCtxItem::Revert),
                        'y' | 'Y' => matches!(it, crate::GitCtxItem::CopyHash),
                        'o' | 'O' | 'b' | 'B' => {
                            matches!(it, crate::GitCtxItem::BrowseOnGitHub)
                        }
                        _ => false,
                    })
                });
                if let Some(idx) = pick {
                    if let Some(m) = app.git_wb.ctx_menu.as_mut() {
                        m.sel = idx;
                    }
                    match app.git_wb.run_ctx_action() {
                        Ok(msg) => {
                            if let Some(h) = msg.strip_prefix("Copied ") {
                                let _ = crate::clipboard::copy(h);
                            }
                            app.message = msg;
                        }
                        Err(e) => app.message = e,
                    }
                }
            }
            _ => {}
        }
        return;
    }

    // Commit message editing (left pane)
    if app.git_wb.commit_editing {
        match code {
            KeyCode::Esc => {
                app.git_wb.commit_editing = false;
                app.message = "Commit message done".into();
            }
            KeyCode::Enter => {
                app.git_wb.commit_editing = false;
                match app.git_wb.commit_with_buf() {
                    Ok(()) => {
                        app.message = app
                            .git_wb
                            .message
                            .clone()
                            .unwrap_or_else(|| "Committed".into());
                        app.refresh_git();
                    }
                    Err(e) => app.message = e,
                }
            }
            KeyCode::Backspace => {
                app.git_wb.commit_buf.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                app.git_wb.commit_buf.push(c);
            }
            _ => {}
        }
        return;
    }

    match code {
        KeyCode::Esc => {
            if app.git_wb.pr_filter_mode {
                app.git_wb.pr_filter_mode = false;
                app.git_wb.pr_filter.clear();
                app.git_wb.refilter_prs();
                app.message = "Filter cleared".into();
                return;
            }
            if app.git_wb.issue_filter_mode {
                app.git_wb.issue_filter_mode = false;
                app.git_wb.issue_filter.clear();
                app.git_wb.refilter_issues();
                app.message = "Filter cleared".into();
                return;
            }
            if app.git_wb.ctx_menu.is_some() {
                app.git_wb.close_ctx_menu();
            } else if !app.git_wb.go_back() {
                app.close_git_workbench();
            }
        }
        // JetBrains dock: Tab cycles Changes | Log | Files panes
        KeyCode::Tab => {
            app.git_wb.cycle_pane();
            app.message = format!("Pane: {:?}", app.git_wb.pane);
        }
        KeyCode::BackTab => app.git_wb.prev_tab(),
        // Number keys switch surfaces. Docked panes: 1=Changes 2=Log 4=Files.
        // 5 opens/focuses Diff for the active column context (no tab thrash).
        KeyCode::Char(d) if d.is_ascii_digit() && d >= '1' && d <= '8' => {
            let n = d as u8 - b'0';
            match n {
                1 => {
                    app.git_wb.tab = GitTab::Status;
                    app.git_wb.pane = crate::git_workbench::GitPane::Changes;
                    app.git_wb.focus = GitFocus::List;
                    app.git_wb.ensure_tab_data();
                    app.message = "Git · Changes".into();
                }
                2 => {
                    app.git_wb.tab = GitTab::History;
                    app.git_wb.pane = crate::git_workbench::GitPane::Log;
                    app.git_wb.focus = GitFocus::List;
                    app.git_wb.ensure_tab_data();
                    app.message = "Git · Log".into();
                }
                3 => {
                    app.git_wb.tab = GitTab::Branches;
                    app.git_wb.focus = GitFocus::List;
                    app.git_wb.ensure_tab_data();
                    app.message = "Git · Branches".into();
                }
                4 => {
                    // Focus docked Files column; load commit detail quietly
                    match app.git_wb.focus_files_pane() {
                        Ok(()) => {
                            app.message = app
                                .git_wb
                                .message
                                .clone()
                                .unwrap_or_else(|| "Git · Files".into());
                        }
                        Err(e) => app.message = e,
                    }
                }
                5 => {
                    // Context-aware Diff from the active docked column.
                    // If already on Diff, stay; else open from Changes/Files/Log.
                    if app.git_wb.tab == GitTab::Diff && app.git_wb.diff_path.is_some() {
                        app.message = "Git · Diff".into();
                    } else {
                        match app.git_wb.open_context_diff() {
                            Ok(()) => {
                                app.message = app
                                    .git_wb
                                    .diff_path
                                    .as_ref()
                                    .map(|p| format!("Diff · {p}"))
                                    .unwrap_or_else(|| "Git · Diff".into());
                            }
                            Err(e) => {
                                // Re-show last diff if one exists
                                if app.git_wb.diff_path.is_some() {
                                    app.git_wb.tab = GitTab::Diff;
                                    app.git_wb.focus = GitFocus::Diff;
                                    app.message = "Git · Diff".into();
                                } else {
                                    app.message = e;
                                }
                            }
                        }
                    }
                }
                6 => {
                    app.git_wb.tab = GitTab::PullRequests;
                    app.git_wb.focus = GitFocus::List;
                    app.git_wb.ensure_tab_data();
                    app.message = "Git · PRs".into();
                }
                7 => {
                    app.git_wb.tab = GitTab::Issues;
                    app.git_wb.focus = GitFocus::List;
                    app.git_wb.ensure_tab_data();
                    app.message = "Git · Issues".into();
                }
                8 => {
                    app.git_wb.tab = GitTab::Auth;
                    app.git_wb.focus = GitFocus::List;
                    app.git_wb.ensure_tab_data();
                    app.message = "Git · Auth".into();
                }
                9 => {
                    app.git_wb.tab = GitTab::Stash;
                    app.git_wb.focus = GitFocus::List;
                    app.git_wb.ensure_tab_data();
                    app.message = "Git · Stash".into();
                }
                _ => {}
            }
        }
        // ── PR / Issue filter typing ─────────────────────
        KeyCode::Char(c) if app.git_wb.pr_filter_mode && app.git_wb.tab == GitTab::PullRequests => {
            if !c.is_control() {
                app.git_wb.pr_filter.push(c);
                app.git_wb.refilter_prs();
            }
            return;
        }
        KeyCode::Backspace
            if app.git_wb.pr_filter_mode && app.git_wb.tab == GitTab::PullRequests =>
        {
            app.git_wb.pr_filter.pop();
            app.git_wb.refilter_prs();
            return;
        }
        KeyCode::Char(c) if app.git_wb.issue_filter_mode && app.git_wb.tab == GitTab::Issues => {
            if !c.is_control() {
                app.git_wb.issue_filter.push(c);
                app.git_wb.refilter_issues();
            }
            return;
        }
        KeyCode::Backspace if app.git_wb.issue_filter_mode && app.git_wb.tab == GitTab::Issues => {
            app.git_wb.issue_filter.pop();
            app.git_wb.refilter_issues();
            return;
        }
        KeyCode::Down | KeyCode::Char('j') => app.git_wb.move_sel(1),
        KeyCode::Up | KeyCode::Char('k') => app.git_wb.move_sel(-1),
        KeyCode::PageDown => app.git_wb.move_sel(10),
        KeyCode::PageUp => app.git_wb.move_sel(-10),
        // JetBrains dock: 'i' edit commit message, 'c' commit
        KeyCode::Char('i')
            if matches!(
                app.git_wb.tab,
                GitTab::Status | GitTab::History | GitTab::Commit
            ) =>
        {
            app.git_wb.commit_editing = true;
            app.git_wb.pane = crate::git_workbench::GitPane::Changes;
            app.message = "Commit message — type, Enter commit, Esc done".into();
        }
        KeyCode::Char('c')
            if !matches!(app.git_wb.tab, GitTab::Branches | GitTab::PullRequests) =>
        {
            match app.git_wb.commit_with_buf() {
                Ok(()) => {
                    app.message = app
                        .git_wb
                        .message
                        .clone()
                        .unwrap_or_else(|| "Committed".into());
                    app.refresh_git();
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('v')
            if matches!(
                app.git_wb.tab,
                GitTab::History | GitTab::Status | GitTab::Commit
            ) =>
        {
            app.git_wb.tab = GitTab::History;
            app.git_wb.toggle_history_view();
            app.message = app
                .git_wb
                .message
                .clone()
                .unwrap_or_else(|| "Toggled history view".into());
        }
        // PR state: [ ] cycle  (also works when empty)
        KeyCode::Char(']') if app.git_wb.tab == GitTab::PullRequests => {
            app.git_wb.cycle_pr_state(true);
            app.message = app.git_wb.message.clone().unwrap_or_default();
        }
        KeyCode::Char('[') if app.git_wb.tab == GitTab::PullRequests => {
            app.git_wb.cycle_pr_state(false);
            app.message = app.git_wb.message.clone().unwrap_or_default();
        }
        KeyCode::Char('s') if app.git_wb.tab == GitTab::Issues => {
            app.git_wb.cycle_issue_state();
            app.message = app.git_wb.message.clone().unwrap_or_default();
        }
        KeyCode::Char('/') if app.git_wb.tab == GitTab::PullRequests => {
            app.git_wb.begin_pr_filter();
            app.message = app.git_wb.message.clone().unwrap_or_default();
        }
        KeyCode::Char('/') if app.git_wb.tab == GitTab::Issues => {
            app.git_wb.begin_issue_filter();
            app.message = app.git_wb.message.clone().unwrap_or_default();
        }
        KeyCode::Enter => match app.git_wb.tab {
            // Docked 3-col: Enter follows the focused column
            GitTab::Status | GitTab::History | GitTab::Commit => {
                use crate::git_workbench::GitPane;
                match app.git_wb.pane {
                    GitPane::Changes => {
                        if let Err(e) = app.git_wb.open_selected_diff() {
                            app.message = e;
                        }
                    }
                    GitPane::Log => {
                        // Load detail + move focus to Files (stay docked)
                        match app.git_wb.focus_files_pane() {
                            Ok(()) => {
                                app.message =
                                    app.git_wb.message.clone().unwrap_or_else(|| "Files".into());
                            }
                            Err(e) => app.message = e,
                        }
                    }
                    GitPane::Files => {
                        if let Err(e) = app.git_wb.open_selected_commit_file_diff() {
                            app.message = e;
                        }
                    }
                }
            }
            GitTab::Branches => match app.git_wb.checkout_selected_branch() {
                Ok(()) => {
                    app.message = app
                        .git_wb
                        .message
                        .clone()
                        .unwrap_or_else(|| "Checked out".into());
                    app.refresh_git();
                }
                Err(e) => app.message = e,
            },
            GitTab::Diff => {}
            GitTab::PullRequests => {
                if app.git_wb.pr_filter_mode {
                    app.git_wb.pr_filter_mode = false;
                    app.message = format!("Filter: {} result(s)", app.git_wb.pr_filtered.len());
                }
            }
            GitTab::Issues => {
                if app.git_wb.issue_filter_mode {
                    app.git_wb.issue_filter_mode = false;
                    app.message = format!("Filter: {} result(s)", app.git_wb.issue_filtered.len());
                } else if let Some(it) = app.git_wb.selected_issue() {
                    let n = it.number.to_string();
                    if let Some(ref root) = app.git_wb.root {
                        match crate::gh::browse(root, Some(&format!("issues/{n}")))
                            .or_else(|_| crate::gh::browse(root, Some(&n)))
                        {
                            Ok(m) => app.message = m,
                            Err(e) => app.message = e,
                        }
                    }
                }
            }
            GitTab::Auth => match app.git_wb.run_auth_action() {
                Ok(()) => {
                    app.message = app.git_wb.message.clone().unwrap_or_else(|| "OK".into());
                }
                Err(e) => app.message = e,
            },
            GitTab::Stash => match app.git_wb.stash_apply_selected() {
                Ok(()) => {
                    app.message = app
                        .git_wb
                        .message
                        .clone()
                        .unwrap_or_else(|| "Stash applied".into());
                    app.refresh_git();
                }
                Err(e) => app.message = e,
            },
        },
        KeyCode::Char('d') if app.git_wb.tab == GitTab::Stash => {
            match app.git_wb.stash_drop_selected() {
                Ok(()) => {
                    app.message = app
                        .git_wb
                        .message
                        .clone()
                        .unwrap_or_else(|| "Stash dropped".into());
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('p') if app.git_wb.tab == GitTab::Stash => {
            match app.git_wb.stash_show_selected() {
                Ok(text) => {
                    // The XLC console is gone; summarise on the status line.
                    app.message = format!("Stash: {} line(s)", text.lines().count());
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('c') if app.git_wb.tab == GitTab::PullRequests => {
            match app.git_wb.checkout_selected_pr() {
                Ok(()) => {
                    app.message = app
                        .git_wb
                        .message
                        .clone()
                        .unwrap_or_else(|| "PR checked out".into());
                    app.refresh_git();
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('M') if app.git_wb.tab == GitTab::PullRequests => {
            match app.git_wb.merge_selected_pr("squash") {
                Ok(()) => {
                    app.message = app
                        .git_wb
                        .message
                        .clone()
                        .unwrap_or_else(|| "Merged".into())
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char(' ') | KeyCode::Char('s') if app.git_wb.tab == GitTab::Status => {
            match app.git_wb.stage_selected() {
                Ok(()) => {
                    app.message = app
                        .git_wb
                        .message
                        .clone()
                        .unwrap_or_else(|| "Staged".into());
                    app.refresh_git();
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('a') if app.git_wb.tab == GitTab::Status => match app.git_wb.stage_all() {
            Ok(()) => {
                app.message = app.git_wb.message.clone().unwrap_or_default();
                app.refresh_git();
            }
            Err(e) => app.message = e,
        },
        KeyCode::Char('A') if app.git_wb.tab == GitTab::Status => match app.git_wb.unstage_all() {
            Ok(()) => {
                app.message = app.git_wb.message.clone().unwrap_or_default();
                app.refresh_git();
            }
            Err(e) => app.message = e,
        },
        KeyCode::Char('x') if app.git_wb.tab == GitTab::Status => {
            if let Err(e) = app.git_wb.begin_discard_selected() {
                app.message = e;
            } else {
                app.message = app.git_wb.message.clone().unwrap_or_default();
            }
        }
        KeyCode::Char('c') if app.git_wb.tab == GitTab::Branches => {
            app.git_wb.begin_new_branch();
            app.message = app.git_wb.message.clone().unwrap_or_default();
        }
        KeyCode::Char('d') if app.git_wb.tab == GitTab::Branches => {
            match app.git_wb.delete_selected_branch() {
                Ok(()) => app.message = app.git_wb.message.clone().unwrap_or_default(),
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('C') if matches!(app.git_wb.tab, GitTab::History | GitTab::Commit) => {
            match app.git_wb.cherry_pick_selected() {
                Ok(()) => {
                    app.message = app.git_wb.message.clone().unwrap_or_default();
                    app.refresh_git();
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('V') if matches!(app.git_wb.tab, GitTab::History | GitTab::Commit) => {
            match app.git_wb.revert_selected() {
                Ok(()) => {
                    app.message = app.git_wb.message.clone().unwrap_or_default();
                    app.refresh_git();
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('y') if matches!(app.git_wb.tab, GitTab::History | GitTab::Commit) => {
            if let Some(h) = app.git_wb.copy_commit_hash() {
                let _ = crate::clipboard::copy(&h);
                app.message = format!("Copied {}", &h[..7.min(h.len())]);
            }
        }
        KeyCode::Char('P') => match app.git_wb.create_pr_from_head() {
            Ok(()) => {
                app.message = app
                    .git_wb
                    .message
                    .clone()
                    .unwrap_or_else(|| "PR created".into())
            }
            Err(e) => app.message = e,
        },
        KeyCode::Char('f') if !matches!(app.git_wb.tab, GitTab::Auth) => {
            // Background — toolbar spinner plays; result lands via poll_loading.
            app.message = app
                .git_wb
                .remote_action(crate::git_workbench::RemoteAction::Fetch);
        }
        KeyCode::Char('p') if !matches!(app.git_wb.tab, GitTab::Auth | GitTab::PullRequests) => {
            app.message = app
                .git_wb
                .remote_action(crate::git_workbench::RemoteAction::Pull);
        }
        KeyCode::Char('R') if !matches!(app.git_wb.tab, GitTab::Auth) => {
            app.message = app
                .git_wb
                .remote_action(crate::git_workbench::RemoteAction::PullRebase);
        }
        KeyCode::Char('u') => {
            app.message = app
                .git_wb
                .remote_action(crate::git_workbench::RemoteAction::Push);
        }
        KeyCode::Char('r') => {
            let hint = app.filename.as_deref();
            if app.git_wb.tab == GitTab::Auth {
                // Async refresh — loading spinner (no UI freeze)
                app.git_wb.refresh_auth();
                app.message = app
                    .git_wb
                    .message
                    .clone()
                    .unwrap_or_else(|| "Refreshing GitHub account…".into());
            } else {
                app.git_wb.refresh(hint);
                if app.git_wb.tab == GitTab::PullRequests {
                    app.git_wb.reload_prs();
                }
                app.message = "Git refreshed".into();
            }
        }
        KeyCode::Char('m') if app.git_wb.tab == GitTab::History => {
            match app.git_wb.load_more_history() {
                Ok(n) => app.message = format!("Loaded +{n} commits"),
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('z') => match app.git_wb.stash() {
            Ok(()) => {
                app.message = app
                    .git_wb
                    .message
                    .clone()
                    .unwrap_or_else(|| "Stashed".into());
                app.refresh_git();
            }
            Err(e) => app.message = e,
        },
        KeyCode::Char('Z') => match app.git_wb.stash_pop() {
            Ok(()) => {
                app.message = app
                    .git_wb
                    .message
                    .clone()
                    .unwrap_or_else(|| "Stash popped".into());
                app.refresh_git();
            }
            Err(e) => app.message = e,
        },
        KeyCode::Char('o') => {
            let r: Result<String, String> =
                if matches!(app.git_wb.tab, GitTab::History | GitTab::Commit) {
                    app.git_wb.browse_commit().map(|()| {
                        app.git_wb
                            .message
                            .clone()
                            .unwrap_or_else(|| "Opened commit".into())
                    })
                } else if app.git_wb.tab == GitTab::PullRequests {
                    if let Some(pr) = app.git_wb.selected_pr() {
                        let n = pr.number.to_string();
                        if let Some(ref root) = app.git_wb.root {
                            crate::gh::browse(root, Some(&n))
                        } else {
                            Err("No git root".into())
                        }
                    } else {
                        app.git_wb.browse_repo().map(|()| {
                            app.git_wb
                                .message
                                .clone()
                                .unwrap_or_else(|| "Browser".into())
                        })
                    }
                } else if app.git_wb.tab == GitTab::Issues {
                    if let Some(it) = app.git_wb.selected_issue() {
                        let n = it.number.to_string();
                        if let Some(ref root) = app.git_wb.root {
                            crate::gh::browse(root, Some(&format!("issues/{n}")))
                                .or_else(|_| crate::gh::browse(root, Some(&n)))
                        } else {
                            Err("No git root".into())
                        }
                    } else {
                        app.git_wb.browse_repo().map(|()| {
                            app.git_wb
                                .message
                                .clone()
                                .unwrap_or_else(|| "Browser".into())
                        })
                    }
                } else {
                    app.git_wb.browse_repo().map(|()| {
                        app.git_wb
                            .message
                            .clone()
                            .unwrap_or_else(|| "Browser".into())
                    })
                };
            match r {
                Ok(msg) => app.message = msg,
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('g') | KeyCode::Char('c')
            if !matches!(app.git_wb.tab, GitTab::Commit | GitTab::Diff | GitTab::Auth) =>
        {
            app.git_wb.from_scm = true;
            app.close_git_workbench();
        }
        _ => {}
    }
}

/// Pretty document preview (Ctrl+Shift+V / :preview / :pr)
fn handle_preview(app: &mut App, code: KeyCode) {
    // While the reverse transform plays, only Esc force-dismisses.
    if app.preview.closing {
        if matches!(code, KeyCode::Esc | KeyCode::Char('q')) {
            app.close_preview_immediate();
            app.message = String::new();
        }
        return;
    }

    // Image: arrow keys resize
    if matches!(app.preview.kind, Some(crate::PreviewKind::Image)) {
        let cell_px = app.cell_px_or_default();
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.close_preview();
                app.message = String::new();
                return;
            }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
                if let Some(img) = app.preview_image.as_mut() {
                    img.adjust_width(-4, cell_px);
                    app.message = format!("Image width {} cells", img.width_cells);
                } else if !app.wrap_lines {
                    // Text preview panning (wrap_lines = false)
                    app.preview.hscroll = app.preview.hscroll.saturating_sub(6);
                }
                return;
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
                if let Some(img) = app.preview_image.as_mut() {
                    img.adjust_width(4, cell_px);
                    app.message = format!("Image width {} cells", img.width_cells);
                } else if !app.wrap_lines {
                    app.preview.hscroll = app.preview.hscroll.saturating_add(6);
                }
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.preview.scroll_by(1, 1);
                return;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.preview.scroll_by(-1, 1);
                return;
            }
            _ => {}
        }
    }

    // Audio: Space toggles playback
    if matches!(app.preview.kind, Some(crate::PreviewKind::Audio)) {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.close_preview();
                app.message = String::new();
                return;
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(player) = app.preview_audio.as_mut() {
                    match player.toggle() {
                        Ok(msg) => {
                            let playing = player.playing();
                            if let Some(ref path) = app.preview.media_path.clone() {
                                app.preview.lines = crate::media::audio_info_lines(path, playing);
                            }
                            app.message = msg;
                        }
                        Err(e) => app.message = e,
                    }
                }
                return;
            }
            _ => {}
        }
    }

    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.close_preview();
            app.message = String::new();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.preview.scroll_by(1, 1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.preview.scroll_by(-1, 1);
        }
        KeyCode::PageDown | KeyCode::Char('f') => {
            app.preview.scroll_by(12, 12);
        }
        KeyCode::PageUp | KeyCode::Char('b') => {
            app.preview.scroll_by(-12, 12);
        }
        KeyCode::Home | KeyCode::Char('g') => {
            app.preview.scroll = 0;
        }
        KeyCode::End | KeyCode::Char('G') => {
            app.preview.scroll = app.preview.lines.len().saturating_sub(1);
        }
        KeyCode::Char('r') => {
            app.refresh_preview_if_open();
            app.message = String::from("Preview refreshed");
        }
        _ => {}
    }
}

/// VS Code Source Control panel (Ctrl+G)
fn handle_scm(app: &mut App, code: KeyCode) {
    use crate::scm::ScmFocus;

    // While sliding out, only Esc force-dismisses.
    if app.scm.closing {
        if code == KeyCode::Esc {
            app.close_scm_immediate();
            app.message = String::new();
        }
        return;
    }
    match code {
        KeyCode::Esc => {
            app.close_scm();
            app.message = String::new();
        }
        // Full Git workbench from light SCM
        KeyCode::Char('G') => {
            app.open_git_workbench();
        }
        KeyCode::Tab => {
            app.scm.cycle_focus(true);
        }
        KeyCode::BackTab => {
            app.scm.cycle_focus(false);
        }
        KeyCode::Enter => match app.scm.focus {
            ScmFocus::Message | ScmFocus::CommitButton => app.scm_commit(),
            ScmFocus::Changes => app.scm_open_selected_file(),
            ScmFocus::Graph => {
                // Flash selected commit detail in status
                if let Some(row) = app.scm.selected_graph_row() {
                    app.message = format!(
                        "{}  {} — {} ({})",
                        row.short, row.subject, row.author, row.when
                    );
                }
            }
        },
        KeyCode::Down | KeyCode::Char('j') if app.scm.focus != ScmFocus::Message => {
            if app.scm.focus == ScmFocus::CommitButton {
                app.scm.focus = ScmFocus::Changes;
            } else if app.scm.focus == ScmFocus::Graph {
                app.scm.move_graph_sel(1);
            } else {
                app.scm.focus = ScmFocus::Changes;
                app.scm.move_sel(1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') if app.scm.focus != ScmFocus::Message => {
            if app.scm.focus == ScmFocus::Changes {
                if app.scm.selected == 0 {
                    app.scm.focus = ScmFocus::CommitButton;
                } else {
                    app.scm.move_sel(-1);
                }
            } else if app.scm.focus == ScmFocus::CommitButton {
                app.scm.focus = ScmFocus::Message;
            } else if app.scm.focus == ScmFocus::Graph {
                if app.scm.graph_selected == 0 {
                    app.scm.focus = ScmFocus::Changes;
                } else {
                    app.scm.move_graph_sel(-1);
                }
            }
        }
        KeyCode::Down if app.scm.focus == ScmFocus::Message => {
            app.scm.focus = ScmFocus::CommitButton;
        }
        KeyCode::Char(' ') | KeyCode::Char('s') if app.scm.focus != ScmFocus::Message => {
            app.scm.focus = ScmFocus::Changes;
            app.scm_stage_selected();
        }
        KeyCode::Char('a') if app.scm.focus != ScmFocus::Message => {
            app.scm_stage_all();
        }
        KeyCode::Char('u') if app.scm.focus != ScmFocus::Message => {
            if let Err(e) = app.scm.unstage_all() {
                app.message = e;
            } else {
                app.message = "Unstaged all".into();
                app.refresh_git();
            }
        }
        KeyCode::Char('r') if app.scm.focus != ScmFocus::Message => {
            app.scm_refresh();
            app.message = "SCM refreshed".into();
        }
        KeyCode::Char('m') | KeyCode::Char('L') if app.scm.focus == ScmFocus::Graph => {
            match app.scm.load_more_graph() {
                Ok(n) => {
                    app.message = format!(
                        "Loaded +{} commits (showing {}, limit {})",
                        n,
                        app.scm.graph.len(),
                        app.scm.graph_limit
                    );
                }
                Err(e) => app.message = e,
            }
        }
        KeyCode::Char('c') if app.scm.focus != ScmFocus::Message => {
            app.scm.focus = ScmFocus::Message;
        }
        KeyCode::Char('x') | KeyCode::Delete if app.scm.focus == ScmFocus::Changes => {
            if let Err(e) = app.scm.discard_selected() {
                app.message = e;
            } else {
                app.message = app
                    .scm
                    .last_result
                    .clone()
                    .unwrap_or_else(|| "Discarded".into());
                app.refresh_git();
            }
        }
        KeyCode::Backspace if app.scm.focus == ScmFocus::Message => {
            app.scm.message.pop();
        }
        KeyCode::Char(ch) if app.scm.focus == ScmFocus::Message && !ch.is_control() => {
            app.scm.message.push(ch);
        }
        KeyCode::Char('g') if app.scm.focus != ScmFocus::Message => {
            app.scm.focus = ScmFocus::Graph;
        }
        _ => {}
    }
}

/// Find-bar key handling. This is a GUI panel that owns the keyboard while
/// it is open — the same shape as `handle_palette`, not a vim mode.
fn handle_search_input(app: &mut App, code: KeyCode) {
    // All of it lives on `App`/`SearchState` now — the handler routes, the
    // inline match-cycling copy it used to carry is gone (A3-1).
    match code {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.commit_search(),
        KeyCode::Backspace => app.search_backspace(),
        KeyCode::Delete => {
            // Same as backspace for a single-line search, except an empty
            // bar is not a cancel.
            if !app.search.input.is_empty() {
                app.search.input.pop();
                app.update_search_input();
            }
        }
        KeyCode::Down => app.search_cycle(true),
        KeyCode::Up => app.search_cycle(false),
        KeyCode::Char(c) => {
            if !c.is_control() {
                app.search_type(c);
            }
        }
        _ => {}
    }
}

fn handle_explorer(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.explorer.close();
            app.mode = Mode::Editor;
        }
        KeyCode::Char('j') | KeyCode::Down => app.explorer.move_down(),
        KeyCode::Char('k') | KeyCode::Up => app.explorer.move_up(),
        KeyCode::Char('h') => {
            if let Some(parent) = app.explorer.cwd.parent().map(|p| p.to_path_buf()) {
                app.explorer.cwd = parent;
                app.explorer.refresh();
            }
        }
        KeyCode::Enter | KeyCode::Char('l') => {
            if let Some(path) = app.explorer.select_current() {
                open_file(app, &path);
            }
        }
        _ => {}
    }
}

fn open_file(app: &mut App, path: &std::path::PathBuf) {
    // Media / data files → pretty preview (images, csv, npy, audio)
    if crate::media::is_media_path(path) {
        app.explorer.close();
        match app.open_media_preview(path) {
            Ok(()) => {}
            Err(e) => {
                app.message = e;
                app.mode = Mode::Editor;
            }
        }
        return;
    }
    let path_str = path.display().to_string();
    app.open_new_tab(&path_str);
    app.explorer.close();
    app.mode = Mode::Editor;
}

/// Handle keys for the Ctrl+Shift+T terminal *window* (not side Ctrl+T mode).
///
/// **Strict PTY policy:** when this returns `true`, the key was fully handled
/// (almost always sent to the child). Returns `false` only for the tiny
/// allowlist that must reach editor chrome (Ctrl+W split chord second key, etc.).
fn handle_pane_terminal_window(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
    if let Some(t) = app.focused_pane_terminal_mut() {
        t.poll();
    }

    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let super_key = modifiers.contains(KeyModifiers::SUPER);

    // Close confirmation dialog owns y/n/Esc
    if app.pane_close_confirm_open() {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.confirm_close_pane_terminal(true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.confirm_close_pane_terminal(false);
            }
            _ => {
                app.message = "Close terminal?  [y]es  /  [n]o · Ctrl+Shift+W cancels".into();
            }
        }
        return true;
    }

    // ── Editor allowlist (escape hatches only) ──────────────────────────
    // Ctrl/Cmd+Shift+T — toggle/close terminal window (macOS face uses Super).
    let chord_shift = shift && (ctrl || super_key);
    if chord_shift && matches!(code, KeyCode::Char('t') | KeyCode::Char('T')) {
        app.toggle_terminal_full();
        return true;
    }
    // Ctrl/Cmd+Shift+W — close confirm
    if chord_shift && matches!(code, KeyCode::Char('w') | KeyCode::Char('W')) {
        app.request_close_pane_terminal();
        return true;
    }
    // Cmd+V / Ctrl+Shift+V — paste clipboard into the child (text or image path).
    // Must precede the Ctrl-byte block below (else Ctrl+Shift+V → literal 0x16).
    // Note: Super+Shift+V is pretty-preview when editor focused; in the terminal
    // pane we always paste (preview is not open here).
    if (super_key || (ctrl && shift)) && matches!(code, KeyCode::Char('v') | KeyCode::Char('V')) {
        paste_clipboard_to_terminal(app);
        return true;
    }
    // Ctrl+W alone — start split chord so user can focus the other pane
    if ctrl
        && !shift
        && !alt
        && !super_key
        && matches!(code, KeyCode::Char('w') | KeyCode::Char('W'))
    {
        app.split.pending_chord = true;
        app.message = String::from("Ctrl+W — (terminal) focus other pane with w");
        return true;
    }
    // Second key of Ctrl+W chord while terminal still focused
    if app.split.pending_chord && !ctrl {
        // Let the normal Ctrl+W chord handler process this (returns false)
        return false;
    }

    // ── Everything else → PTY ───────────────────────────────────────────
    // From here on every key belongs to THIS pane's shell. Each terminal pane
    // owns its own process, so the write target is the focused pane's, never a
    // single shared `App.terminal`.
    let Some(term) = app.focused_pane_terminal_mut() else {
        return true;
    };
    // Ctrl+C / Ctrl+D / Ctrl+Z / Ctrl+L … as real control bytes
    if ctrl && !super_key {
        if let KeyCode::Char(c) = code {
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                let byte = (lower as u8) - b'a' + 1;
                term.write_input(&[byte]);
                return true;
            }
        }
        // Ctrl+Arrow etc. — still useful in some REPLs
        match code {
            KeyCode::Left => {
                term.write_input(b"\x1b[1;5D");
                return true;
            }
            KeyCode::Right => {
                term.write_input(b"\x1b[1;5C");
                return true;
            }
            KeyCode::Up => {
                term.write_input(b"\x1b[1;5A");
                return true;
            }
            KeyCode::Down => {
                term.write_input(b"\x1b[1;5B");
                return true;
            }
            _ => {}
        }
    }

    // Alt+char → ESC + char (readline / fish bindings)
    if alt && !ctrl {
        if let KeyCode::Char(c) = code {
            let mut buf = [0u8; 8];
            buf[0] = 0x1b;
            let s = c.encode_utf8(&mut buf[1..]);
            let n = 1 + s.len();
            term.write_input(&buf[..n]);
            return true;
        }
    }

    write_terminal_key(term, apply_shift_for_pty(code, shift));
    true
}

/// Re-apply Shift to a letter on its way to the PTY.
///
/// Core's key model has no separate uppercase key: the face lowercases letters
/// and carries the case as `KeyModifiers::SHIFT`, which is what the editor's
/// own bindings expect. `write_terminal_key` only ever saw the `KeyCode`, so
/// the shell was handed the lowercased character and `echo HELLO` arrived as
/// `echo hello`. Non-letters need no help — they cross with their real glyph.
fn apply_shift_for_pty(code: KeyCode, shift: bool) -> KeyCode {
    match code {
        KeyCode::Char(c) if shift && c.is_lowercase() => {
            let mut up = c.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) => KeyCode::Char(u),
                _ => code,
            }
        }
        other => other,
    }
}

fn write_terminal_key(term: &mut crate::term::Terminal, code: KeyCode) {
    match code {
        KeyCode::Enter => term.write_input(b"\r"),
        KeyCode::Backspace => term.write_input(&[0x7f]),
        KeyCode::Tab => term.write_input(b"\t"),
        // Arrows honor DECCKM (vim/less switch to application cursor keys).
        KeyCode::Left => {
            let seq = term.arrow_seq('D');
            term.write_input(seq);
        }
        KeyCode::Right => {
            let seq = term.arrow_seq('C');
            term.write_input(seq);
        }
        KeyCode::Up => {
            let seq = term.arrow_seq('A');
            term.write_input(seq);
        }
        KeyCode::Down => {
            let seq = term.arrow_seq('B');
            term.write_input(seq);
        }
        KeyCode::Home => term.write_input(b"\x1b[H"),
        KeyCode::End => term.write_input(b"\x1b[F"),
        KeyCode::PageUp => term.scroll_up(3),
        KeyCode::PageDown => term.scroll_down(3),
        KeyCode::Delete => term.write_input(b"\x1b[3~"),
        KeyCode::Esc => term.write_input(b"\x1b"),
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            term.write_input(s.as_bytes());
        }
        _ => {}
    }
}

/// Side-panel terminal (Ctrl+T) — still Mode::Terminal.
fn handle_terminal(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.terminal.poll();

    match code {
        KeyCode::Esc => {
            // Side terminal: Esc closes. Pane terminals are tabs now and
            // handled by `handle_pane_terminal_window` before this runs.
            app.terminal.open = false;
            app.terminal.shutdown();
            app.mode = Mode::Editor;
        }
        other => write_terminal_key(
            &mut app.terminal,
            apply_shift_for_pty(other, modifiers.contains(KeyModifiers::SHIFT)),
        ),
    }
}

fn trigger_completion(app: &mut App) {
    let prefix = word_before_cursor(app);
    let ext = app.file_extension();
    app.completions.activate(&prefix, ext.as_deref());
    if app.lsp.server_running {
        // Flush pending edits first so completions are computed at the
        // position the user actually sees.
        app.sync_lsp_document();
        if let Some(ref path) = app.filename {
            let c = app.buffer.cursor();
            app.lsp
                .request_completion(&path.display().to_string(), c.row, c.col);
        }
    }
}

fn word_before_cursor(app: &App) -> String {
    let cursor = app.buffer.cursor();
    let line = app.buffer.line(cursor.row);
    let chars: Vec<char> = line.chars().collect();

    let mut start = cursor.col;
    while start > 0 {
        let c = chars[start - 1];
        if c.is_alphanumeric() || c == '_' {
            start -= 1;
        } else {
            break;
        }
    }

    chars[start..cursor.col].iter().collect()
}
