//! Call hierarchy panel state (LSP-backed).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallDirection {
    Incoming,
    Outgoing,
}

impl CallDirection {
    pub fn label(self) -> &'static str {
        match self {
            CallDirection::Incoming => "incoming",
            CallDirection::Outgoing => "outgoing",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            CallDirection::Incoming => CallDirection::Outgoing,
            CallDirection::Outgoing => CallDirection::Incoming,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CallItem {
    pub name: String,
    pub detail: String,
    pub kind: String,
    pub path: String,
    pub row: usize,
    pub col: usize,
    /// Full CallHierarchyItem JSON for follow-up requests.
    pub raw_json: String,
}

#[derive(Debug, Clone)]
pub struct CallHierarchyState {
    pub open: bool,
    pub direction: CallDirection,
    pub root_name: String,
    pub items: Vec<CallItem>,
    pub selected: usize,
    pub loading: bool,
    pub message: String,
}

impl Default for CallHierarchyState {
    fn default() -> Self {
        Self {
            open: false,
            direction: CallDirection::Incoming,
            root_name: String::new(),
            items: Vec::new(),
            selected: 0,
            loading: false,
            message: String::new(),
        }
    }
}

impl CallHierarchyState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.loading = false;
        self.message.clear();
    }

    pub fn begin(&mut self, root_name: &str, direction: CallDirection) {
        self.open = true;
        self.direction = direction;
        self.root_name = root_name.to_string();
        self.items.clear();
        self.selected = 0;
        self.loading = true;
        self.message = format!("Call hierarchy ({})…", direction.label());
    }

    pub fn set_items(&mut self, items: Vec<CallItem>) {
        self.items = items;
        self.selected = 0;
        self.loading = false;
        self.message = if self.items.is_empty() {
            format!("No {} calls", self.direction.label())
        } else {
            format!(
                "{} · {} {} call(s)",
                self.root_name,
                self.items.len(),
                self.direction.label()
            )
        };
    }

    pub fn move_sel(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as isize;
        let cur = self.selected as isize + delta;
        self.selected = cur.rem_euclid(n) as usize;
    }

    pub fn selected_item(&self) -> Option<&CallItem> {
        self.items.get(self.selected)
    }
}
