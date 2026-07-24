//! Insert-mode snippets (v1) — Tab expands a trigger word before the cursor.

/// Expand snippet at cursor if the word before cursor matches a trigger.
/// Returns `Some(message)` when expanded.
pub fn try_expand(buffer: &mut crate::buffer::Buffer, ext: Option<&str>) -> Option<String> {
    let row = buffer.cursor.row;
    let col = buffer.cursor.col;
    let line = buffer.line(row);
    let chars: Vec<char> = line.chars().collect();
    if col == 0 || col > chars.len() {
        return None;
    }
    // Word = [A-Za-z0-9_]+ before cursor
    let mut start = col;
    while start > 0 {
        let c = chars[start - 1];
        if c.is_ascii_alphanumeric() || c == '_' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == col {
        return None;
    }
    let trigger: String = chars[start..col].iter().collect();
    let body = lookup(&trigger, ext)?;
    // Delete trigger
    buffer.cursor.col = start;
    for _ in 0..(col - start) {
        if buffer.cursor.col < buffer.line(row).chars().count() {
            buffer.delete_char_at_cursor();
        }
    }
    // Insert body; $0 = final cursor, $1 ignored for v1 (just stripped)
    let (text, cursor_off) = expand_body(body);
    let insert_row = buffer.cursor.row;
    let insert_col = buffer.cursor.col;
    buffer.insert_str(&text);
    // Place cursor at $0 position
    let (r, c) = offset_to_row_col(&text, cursor_off, insert_row, insert_col);
    buffer.cursor.row = r;
    buffer.cursor.col = c;
    buffer.clamp_col();
    Some(format!("snippet · {trigger}"))
}

fn lookup(trigger: &str, ext: Option<&str>) -> Option<&'static str> {
    let lang = ext.unwrap_or("");
    // Language-specific first
    let specific = match lang {
        "rs" => match trigger {
            "fn" => Some("fn ${1:name}($2) {\n    $0\n}"),
            "pfn" => Some("pub fn ${1:name}($2) {\n    $0\n}"),
            "impl" => Some("impl ${1:Type} {\n    $0\n}"),
            "struct" => Some("struct ${1:Name} {\n    $0\n}"),
            "enum" => Some("enum ${1:Name} {\n    $0\n}"),
            "match" => Some("match ${1:expr} {\n    $2 => $0,\n}"),
            "test" => Some("#[test]\nfn ${1:it_works}() {\n    $0\n}"),
            "der" => Some("#[derive(Debug, Clone)]\n"),
            _ => None,
        },
        "py" | "pyi" => match trigger {
            "def" => Some("def ${1:name}($2):\n    $0"),
            "class" => Some("class ${1:Name}:\n    $0"),
            "if" => Some("if ${1:cond}:\n    $0"),
            "for" => Some("for ${1:i} in ${2:iterable}:\n    $0"),
            "main" => Some("if __name__ == \"__main__\":\n    $0"),
            _ => None,
        },
        "js" | "jsx" | "ts" | "tsx" | "mjs" => match trigger {
            "fn" | "function" => Some("function ${1:name}($2) {\n  $0\n}"),
            "af" => Some("const ${1:name} = ($2) => {\n  $0\n}"),
            "log" => Some("console.log($0);"),
            "for" => Some("for (let ${1:i} = 0; $1 < ${2:n}; $1++) {\n  $0\n}"),
            _ => None,
        },
        "go" => match trigger {
            "fn" | "func" => Some("func ${1:name}($2) {\n\t$0\n}"),
            "main" => Some("func main() {\n\t$0\n}"),
            _ => None,
        },
        _ => None,
    };
    if specific.is_some() {
        return specific;
    }
    // Generic
    match trigger {
        "if" => Some("if ${1:cond} {\n    $0\n}"),
        "for" => Some("for ${1:i} {\n    $0\n}"),
        "todo" => Some("// TODO: $0"),
        "fix" => Some("// FIXME: $0"),
        _ => None,
    }
}

/// Expand `$0` / `$1`… placeholders. Returns (text, byte-ish char offset of $0).
fn expand_body(body: &str) -> (String, usize) {
    let mut out = String::new();
    let mut cursor_at = 0usize;
    let mut chars = body.chars().peekable();
    let mut i = 0usize;
    while let Some(c) = chars.next() {
        if c == '$' {
            if chars.peek() == Some(&'{') {
                chars.next(); // {
                let mut name = String::new();
                while let Some(&ch) = chars.peek() {
                    chars.next();
                    if ch == '}' {
                        break;
                    }
                    name.push(ch);
                }
                // ${1:default} or ${0}
                let (idx, default) = if let Some((a, b)) = name.split_once(':') {
                    (a, b)
                } else {
                    (name.as_str(), "")
                };
                if idx == "0" {
                    cursor_at = i;
                    // no default insert for $0
                } else {
                    for ch in default.chars() {
                        out.push(ch);
                        i += 1;
                    }
                }
            } else if chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                let mut num = String::new();
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    num.push(chars.next().unwrap());
                }
                if num == "0" {
                    cursor_at = i;
                }
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    if cursor_at > i {
        cursor_at = i;
    }
    (out, cursor_at)
}

fn offset_to_row_col(text: &str, off: usize, base_row: usize, base_col: usize) -> (usize, usize) {
    let mut r = base_row;
    let mut c = base_col;
    for (i, ch) in text.chars().enumerate() {
        if i >= off {
            break;
        }
        if ch == '\n' {
            r += 1;
            c = 0;
        } else {
            c += 1;
        }
    }
    (r, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_strips_placeholders() {
        let (t, off) = expand_body("fn ${1:name}() {\n    $0\n}");
        assert!(t.contains("fn name()"));
        assert!(!t.contains('$'));
        assert!(off > 0);
    }
}
