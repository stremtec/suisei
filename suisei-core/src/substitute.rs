//! Ex-style substitute: `:s/pat/repl/flags` and `:%s/.../`

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubstituteCmd {
    pub pattern: String,
    pub replacement: String,
    /// whole file (`%s`)
    pub global_file: bool,
    /// all occurrences on each line (`g` flag)
    pub global_line: bool,
    pub confirm: bool,
}

/// Parse `s/pat/repl/flags`, `%s/pat/repl/g`, `s#pat#repl#g`
pub fn parse_substitute(input: &str) -> Option<SubstituteCmd> {
    let input = input.trim();
    let (global_file, rest) = if let Some(r) = input.strip_prefix("%s") {
        (true, r.trim_start())
    } else if let Some(r) = input.strip_prefix('s') {
        // not `set` / `save`
        if r.starts_with(|c: char| c.is_alphanumeric()) {
            return None;
        }
        (false, r.trim_start())
    } else {
        return None;
    };

    if rest.is_empty() {
        return None;
    }
    let delim = rest.chars().next()?;
    if delim.is_whitespace() {
        return None;
    }
    let rest = &rest[delim.len_utf8()..];

    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            cur.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == delim {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }

    // `s/foo/bar/g` → parts=[foo, bar], cur="g" (flags)
    // `s/foo/bar/`  → parts=[foo, bar] or [foo, bar, ""], cur=""
    let flags = if parts.len() >= 2 {
        if parts.len() >= 3 {
            // flags after third delim empty segment
            let mut f = parts[2].clone();
            f.push_str(&cur);
            f
        } else {
            cur
        }
    } else {
        // only one part so far — treat leftover as replacement
        if !cur.is_empty() {
            parts.push(cur);
        }
        String::new()
    };

    let pattern = parts.first()?.clone();
    if pattern.is_empty() {
        return None;
    }
    let replacement = parts.get(1).cloned().unwrap_or_default();
    let global_line = flags.contains('g');
    let confirm = flags.contains('c');

    Some(SubstituteCmd {
        pattern,
        replacement,
        global_file,
        global_line,
        confirm,
    })
}

/// Apply substitute. Returns (new_lines, substitution count).
pub fn apply_substitute(
    lines: &[String],
    cmd: &SubstituteCmd,
    cursor_row: usize,
) -> (Vec<String>, usize) {
    let mut count = 0usize;
    let mut out = lines.to_vec();
    if out.is_empty() {
        return (out, 0);
    }

    let rows: Vec<usize> = if cmd.global_file {
        (0..out.len()).collect()
    } else {
        vec![cursor_row.min(out.len() - 1)]
    };

    for row in rows {
        let (new_line, n) =
            replace_on_line(&out[row], &cmd.pattern, &cmd.replacement, cmd.global_line);
        if n > 0 {
            out[row] = new_line;
            count += n;
        }
    }
    (out, count)
}

fn replace_on_line(line: &str, pat: &str, repl: &str, all: bool) -> (String, usize) {
    if pat.is_empty() {
        return (line.to_string(), 0);
    }
    if all {
        let n = line.matches(pat).count();
        (line.replace(pat, repl), n)
    } else if let Some(idx) = line.find(pat) {
        let mut s = String::with_capacity(line.len() + repl.len());
        s.push_str(&line[..idx]);
        s.push_str(repl);
        s.push_str(&line[idx + pat.len()..]);
        (s, 1)
    } else {
        (line.to_string(), 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let c = parse_substitute("s/foo/bar/").unwrap();
        assert_eq!(c.pattern, "foo");
        assert_eq!(c.replacement, "bar");
        assert!(!c.global_file);
        assert!(!c.global_line);
    }

    #[test]
    fn parse_global() {
        let c = parse_substitute("%s/a/b/g").unwrap();
        assert!(c.global_file);
        assert!(c.global_line);
        assert_eq!(c.pattern, "a");
        assert_eq!(c.replacement, "b");
    }

    #[test]
    fn apply_line() {
        let lines = vec!["foo bar foo".into()];
        let cmd = parse_substitute("s/foo/X/").unwrap();
        let (out, n) = apply_substitute(&lines, &cmd, 0);
        assert_eq!(n, 1);
        assert_eq!(out[0], "X bar foo");
    }

    #[test]
    fn apply_global() {
        let lines = vec!["a a".into(), "a".into()];
        let cmd = parse_substitute("%s/a/b/g").unwrap();
        let (out, n) = apply_substitute(&lines, &cmd, 0);
        assert_eq!(n, 3);
        assert_eq!(out[0], "b b");
        assert_eq!(out[1], "b");
    }
}
