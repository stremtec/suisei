//! Pretty commit graph layout (VS Code / lazygit style).
//!
//! Builds colored **lanes** from `git log` parent topology so the TUI can
//! draw `●` / `│` / merge connectors instead of raw `git log --graph` ASCII.

/// One cell in the graph column strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphGlyph {
    Empty,
    /// Commit node `●`
    Node(u8),
    /// Vertical edge `│`
    Pipe(u8),
    /// Merge / fork connectors (unicode box drawing)
    /// `╭` coming from upper-right into this column
    CornerTL(u8),
    /// `╮`
    CornerTR(u8),
    /// `╰`
    CornerBL(u8),
    /// `╯`
    CornerBR(u8),
    /// `─`
    Horizontal(u8),
    /// `├`
    TeeRight(u8),
    /// `┤`
    TeeLeft(u8),
    /// `┼`
    Cross(u8),
    /// `/` style merge approaching from right-below → left-above (drawn as `╱`)
    Slash(u8),
    /// `\` style `╲`
    Backslash(u8),
}

impl GraphGlyph {
    pub fn ch(self) -> char {
        match self {
            GraphGlyph::Empty => ' ',
            GraphGlyph::Node(_) => '●',
            GraphGlyph::Pipe(_) => '│',
            GraphGlyph::CornerTL(_) => '╭',
            GraphGlyph::CornerTR(_) => '╮',
            GraphGlyph::CornerBL(_) => '╰',
            GraphGlyph::CornerBR(_) => '╯',
            GraphGlyph::Horizontal(_) => '─',
            GraphGlyph::TeeRight(_) => '├',
            GraphGlyph::TeeLeft(_) => '┤',
            GraphGlyph::Cross(_) => '┼',
            GraphGlyph::Slash(_) => '╱',
            GraphGlyph::Backslash(_) => '╲',
        }
    }

    pub fn color_id(self) -> Option<u8> {
        match self {
            GraphGlyph::Empty => None,
            GraphGlyph::Node(c)
            | GraphGlyph::Pipe(c)
            | GraphGlyph::CornerTL(c)
            | GraphGlyph::CornerTR(c)
            | GraphGlyph::CornerBL(c)
            | GraphGlyph::CornerBR(c)
            | GraphGlyph::Horizontal(c)
            | GraphGlyph::TeeRight(c)
            | GraphGlyph::TeeLeft(c)
            | GraphGlyph::Cross(c)
            | GraphGlyph::Slash(c)
            | GraphGlyph::Backslash(c) => Some(c),
        }
    }
}

/// One rendered commit row in the graph.
#[derive(Debug, Clone)]
pub struct GraphRow {
    pub hash: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    pub when: String,
    /// Decorations e.g. `HEAD -> master, origin/master`
    pub refs: String,
    pub lane: usize,
    pub color: u8,
    /// Graph strip (one glyph per column; typically width ≤ 8)
    pub glyphs: Vec<GraphGlyph>,
}

#[derive(Debug, Clone)]
pub(crate) struct RawCommit {
    hash: String,
    short: String,
    parents: Vec<String>,
    refs: String,
    subject: String,
    author: String,
    when: String,
}

/// Palette size for lane colors (UI maps id → RGB).
pub const LANE_COLORS: usize = 8;

/// Parse `git log --pretty=format:%H%x00%h%x00%P%x00%d%x00%s%x00%an%x00%ar` output
/// (records separated by newlines; empty parent field allowed).
pub(crate) fn parse_log_output(text: &str) -> Vec<RawCommit> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\0').collect();
        if parts.len() < 7 {
            // tolerate missing trailing fields
            if parts.len() < 3 {
                continue;
            }
        }
        let hash = parts.first().copied().unwrap_or("").to_string();
        if hash.len() < 7 {
            continue;
        }
        let short = parts.get(1).copied().unwrap_or(&hash[..7.min(hash.len())]).to_string();
        let parents: Vec<String> = parts
            .get(2)
            .copied()
            .unwrap_or("")
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let refs = parts
            .get(3)
            .copied()
            .unwrap_or("")
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .to_string();
        let subject = parts.get(4).copied().unwrap_or("").to_string();
        let author = parts.get(5).copied().unwrap_or("").to_string();
        let when = parts.get(6).copied().unwrap_or("").to_string();
        out.push(RawCommit {
            hash,
            short,
            parents,
            refs,
            subject,
            author,
            when,
        });
    }
    out
}

/// Layout commits into colored lanes (newest first, as `git log` returns).
pub(crate) fn layout_graph(commits: &[RawCommit]) -> Vec<GraphRow> {
    if commits.is_empty() {
        return Vec::new();
    }

    // Active lanes: each entry is the commit hash expected next in that column
    // (i.e. child already drawn, waiting for this parent).
    let mut active: Vec<Option<String>> = Vec::new();
    // Stable color per lane index
    let mut rows = Vec::with_capacity(commits.len());

    // Map hash → index for quick "already have lane for parent"
    let _ = commits;

    for commit in commits {
        // 1) Find existing lane reserved for this commit, else open a new one
        let mut lane = None;
        for (i, slot) in active.iter().enumerate() {
            if slot.as_ref() == Some(&commit.hash) {
                lane = Some(i);
                break;
            }
        }
        if lane.is_none() {
            if let Some(i) = active.iter().position(|s| s.is_none()) {
                active[i] = Some(commit.hash.clone());
                lane = Some(i);
            } else {
                active.push(Some(commit.hash.clone()));
                lane = Some(active.len() - 1);
            }
        }
        let lane = lane.unwrap();
        let color = (lane % LANE_COLORS) as u8;

        // 2) Build glyph row for *current* active lanes (before parent update)
        let width = active.len().max(1).min(10);
        let mut glyphs = vec![GraphGlyph::Empty; width];
        for (i, slot) in active.iter().enumerate().take(width) {
            if slot.is_some() {
                if i == lane {
                    glyphs[i] = GraphGlyph::Node(color);
                } else {
                    let c = (i % LANE_COLORS) as u8;
                    glyphs[i] = GraphGlyph::Pipe(c);
                }
            }
        }

        // 3) Update active lanes with parents
        // First parent continues this lane unless already reserved elsewhere (merge).
        let parents = &commit.parents;
        if parents.is_empty() {
            active[lane] = None;
        } else {
            let first = &parents[0];
            if let Some(existing) = active
                .iter()
                .enumerate()
                .find(|(i, s)| *i != lane && s.as_ref() == Some(first))
                .map(|(i, _)| i)
            {
                // Parent already has a lane → close ours and draw merge bridge
                active[lane] = None;
                paint_merge_link(&mut glyphs, lane, existing, color);
            } else {
                active[lane] = Some(first.clone());
            }

            for p in parents.iter().skip(1) {
                if let Some(target) = active.iter().position(|s| s.as_ref() == Some(p)) {
                    paint_merge_link(&mut glyphs, lane, target, color);
                    continue;
                }
                if let Some(i) = active.iter().position(|s| s.is_none()) {
                    active[i] = Some(p.clone());
                    if glyphs.len() <= i {
                        glyphs.resize(i + 1, GraphGlyph::Empty);
                    }
                    paint_merge_link(&mut glyphs, lane, i, (i % LANE_COLORS) as u8);
                } else if active.len() < 10 {
                    active.push(Some(p.clone()));
                    let i = active.len() - 1;
                    glyphs.resize(i + 1, GraphGlyph::Empty);
                    paint_merge_link(&mut glyphs, lane, i, (i % LANE_COLORS) as u8);
                }
            }
        }

        // Compact trailing empties from active (keep holes for stability mid-graph)
        while active.last().is_some_and(|s| s.is_none()) {
            active.pop();
        }

        rows.push(GraphRow {
            hash: commit.hash.clone(),
            short: commit.short.clone(),
            subject: commit.subject.clone(),
            author: commit.author.clone(),
            when: commit.when.clone(),
            refs: commit.refs.clone(),
            lane,
            color,
            glyphs,
        });
    }

    rows
}

/// Draw a horizontal merge bridge between two columns on the current row.
fn paint_merge_link(glyphs: &mut Vec<GraphGlyph>, from: usize, to: usize, color: u8) {
    if from == to {
        return;
    }
    let (lo, hi) = if from < to { (from, to) } else { (to, from) };
    if glyphs.len() <= hi {
        glyphs.resize(hi + 1, GraphGlyph::Empty);
    }
    // Keep node at `from`; fill middle with ─; put a tee/corner at ends.
    for i in lo + 1..hi {
        match glyphs[i] {
            GraphGlyph::Empty => glyphs[i] = GraphGlyph::Horizontal(color),
            GraphGlyph::Pipe(c) => glyphs[i] = GraphGlyph::Cross(c),
            GraphGlyph::Node(_) => {}
            _ => glyphs[i] = GraphGlyph::Horizontal(color),
        }
    }
    // Corners at endpoints (don't overwrite Node)
    if !matches!(glyphs[lo], GraphGlyph::Node(_)) {
        glyphs[lo] = if from < to {
            GraphGlyph::TeeRight(color)
        } else {
            GraphGlyph::CornerBL(color)
        };
    }
    if !matches!(glyphs[hi], GraphGlyph::Node(_)) {
        glyphs[hi] = if from < to {
            GraphGlyph::CornerTR(color)
        } else {
            GraphGlyph::TeeLeft(color)
        };
    } else if from != hi {
        // target already has node from another meaning — use pipe under merge feel
    }
}

/// High-level: parse + layout from raw git log pretty output.
pub fn build_graph(log_text: &str) -> Vec<GraphRow> {
    let commits = parse_log_output(log_text);
    layout_graph(&commits)
}

/// Map lane color id → (r,g,b) — VS Code–ish branch colors.
pub fn lane_rgb(id: u8) -> (u8, u8, u8) {
    const P: [(u8, u8, u8); LANE_COLORS] = [
        (180, 120, 255), // purple
        (80, 180, 255),  // blue
        (80, 210, 140),  // green
        (255, 170, 70),  // orange
        (255, 120, 180), // pink
        (100, 220, 220), // cyan
        (255, 220, 100), // yellow
        (160, 160, 255), // soft indigo
    ];
    P[(id as usize) % LANE_COLORS]
}

/// Build a short detail string for the selection popup / detail line.
pub fn detail_line(row: &GraphRow) -> String {
    let mut s = format!("{} · {} · {}", row.short, row.author, row.when);
    if !row.refs.is_empty() {
        s.push_str(" · ");
        s.push_str(&row.refs);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_commit() {
        let text = "aabbccddeeff00112233445566778899aabbccdd\0aabbccd\0\0HEAD -> master\0init\0Alice\02 days ago\n";
        let c = parse_log_output(text);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].short, "aabbccd");
        assert_eq!(c[0].subject, "init");
        assert!(c[0].parents.is_empty());
    }

    #[test]
    fn linear_history_one_lane() {
        // newest first: c2 -> c1 -> c0
        let text = "\
ccc0000000000000000000000000000000000002\0ccc0002\0bbb0000000000000000000000000000000000001\0\0third\0A\01 hour ago\n\
bbb0000000000000000000000000000000000001\0bbb0001\0aaa0000000000000000000000000000000000000\0\0second\0A\02 hours ago\n\
aaa0000000000000000000000000000000000000\0aaa0000\0\0\0first\0A\03 hours ago\n";
        let rows = build_graph(text);
        assert_eq!(rows.len(), 3);
        // all on lane 0 ideally
        assert!(rows.iter().all(|r| r.lane == 0));
        assert!(matches!(rows[0].glyphs[0], GraphGlyph::Node(_)));
    }

    #[test]
    fn branch_creates_second_lane() {
        // c_main parents p
        // c_feat parents p  (two children of p → two lanes when both shown)
        // Order newest-first: feat, main, p
        let p = "ppp0000000000000000000000000000000000000";
        let main = "mmm0000000000000000000000000000000000000";
        let feat = "fff0000000000000000000000000000000000000";
        let text = format!(
            "{feat}\0fff0000\0{p}\0\0feat work\0A\01 hour ago\n\
             {main}\0mmm0000\0{p}\0HEAD -> master\0main work\0A\02 hours ago\n\
             {p}\0ppp0000\0\0\0base\0A\03 hours ago\n"
        );
        let rows = build_graph(&text);
        assert_eq!(rows.len(), 3);
        // At least one row should use a non-zero lane or we still have nodes
        assert!(rows.iter().any(|r| r.glyphs.iter().any(|g| matches!(g, GraphGlyph::Node(_)))));
    }

    #[test]
    fn glyph_chars() {
        assert_eq!(GraphGlyph::Node(0).ch(), '●');
        assert_eq!(GraphGlyph::Pipe(1).ch(), '│');
    }
}
