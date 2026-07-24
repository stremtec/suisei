use std::path::PathBuf;

pub struct Explorer {
    pub open: bool,
    pub cwd: PathBuf,
    pub entries: Vec<ExplorerEntry>,
    pub selected: usize,
    pub scroll: usize,
}

#[derive(Clone, Debug)]
pub struct ExplorerEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

impl Default for Explorer {
    fn default() -> Self {
        Self {
            open: false,
            cwd: std::env::current_dir().unwrap_or_default(),
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
        }
    }
}

impl Explorer {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub fn toggle(&mut self, anchor_path: Option<&PathBuf>) {
        self.open = !self.open;
        if self.open {
            if let Some(path) = anchor_path {
                if let Some(parent) = path.parent() {
                    self.cwd = parent.to_path_buf();
                }
            }
            self.refresh();
        }
    }

    pub fn toggle_at(&mut self, anchor: Option<&PathBuf>) {
        self.open = true;
        if self.entries.is_empty() {
            if let Some(path) = anchor {
                if let Some(parent) = path.parent() {
                    self.cwd = parent.to_path_buf();
                }
            }
        }
        self.refresh();
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        if let Ok(dir) = std::fs::read_dir(&self.cwd) {
            let mut dirs: Vec<ExplorerEntry> = Vec::new();
            let mut files: Vec<ExplorerEntry> = Vec::new();

            for entry in dir.flatten() {
                let path = entry.path();
                let is_dir = path.is_dir();
                let name = entry.file_name().to_string_lossy().to_string();

                if name.starts_with('.') && name != ".." {
                    continue;
                }

                let e = ExplorerEntry { name, path, is_dir };
                if is_dir {
                    dirs.push(e);
                } else {
                    files.push(e);
                }
            }

            dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

            if self.cwd.parent().is_some() {
                self.entries.push(ExplorerEntry {
                    name: "..".to_string(),
                    path: self.cwd.parent().unwrap().to_path_buf(),
                    is_dir: true,
                });
            }

            self.entries.extend(dirs);
            self.entries.extend(files);
        }
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
        // Keep selected row in view; callers should set visible_height via ensure_visible.
        self.ensure_visible(24);
    }

    /// Scroll so that `selected` stays within a window of `visible` rows.
    pub fn ensure_visible(&mut self, visible: usize) {
        let visible = visible.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected - visible + 1;
        }
    }

    pub fn select_current(&mut self) -> Option<PathBuf> {
        if let Some(entry) = self.entries.get(self.selected) {
            if entry.is_dir {
                self.cwd = entry.path.clone();
                self.refresh();
                None
            } else {
                Some(entry.path.clone())
            }
        } else {
            None
        }
    }
}
