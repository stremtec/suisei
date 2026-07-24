//! Token kinds, theme styling, and line-based fallback tokenizer.

use ratatui::style::{Modifier, Style};

use crate::theme::Theme;

/// Rich highlight categories (tree-sitter captures + LSP semantic tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    KeywordControl,
    KeywordImport,
    String,
    Comment,
    Number,
    TypeName,
    Function,
    Method,
    Macro,
    Namespace,
    Parameter,
    Property,
    Constant,
    Variable,
    Operator,
    Punctuation,
    Attribute,
    Lifetime,
}

pub fn style_for(theme: &Theme, kind: TokenKind) -> Style {
    let (fg, bold) = match kind {
        TokenKind::Keyword | TokenKind::KeywordControl | TokenKind::KeywordImport => {
            (theme.keyword, true)
        }
        TokenKind::String => (theme.string, false),
        TokenKind::Comment => (theme.comment, false),
        TokenKind::Number => (theme.number, false),
        TokenKind::TypeName => (theme.type_name, false),
        TokenKind::Function => (theme.function, false),
        TokenKind::Method => (theme.function, false),
        TokenKind::Macro => (theme.macro_name, false),
        TokenKind::Namespace => (theme.namespace, false),
        TokenKind::Parameter => (theme.parameter, false),
        TokenKind::Property => (theme.property, false),
        TokenKind::Constant => (theme.constant, false),
        TokenKind::Variable => (theme.fg, false),
        TokenKind::Operator => (theme.operator, false),
        TokenKind::Punctuation => (theme.punctuation, false),
        TokenKind::Attribute => (theme.macro_name, false),
        TokenKind::Lifetime => (theme.parameter, false),
    };
    let mut s = Style::default().fg(fg);
    if bold {
        s = s.add_modifier(Modifier::BOLD);
    }
    if matches!(kind, TokenKind::Comment) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    s
}

/// Map LSP semantic token type name → TokenKind
pub fn from_semantic_type(name: &str) -> TokenKind {
    match name {
        "namespace" | "module" => TokenKind::Namespace,
        "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => TokenKind::TypeName,
        "parameter" => TokenKind::Parameter,
        "variable" => TokenKind::Variable,
        "property" | "enumMember" | "event" => TokenKind::Property,
        "function" => TokenKind::Function,
        "method" => TokenKind::Method,
        "macro" | "decorator" => TokenKind::Macro,
        "keyword" | "modifier" => TokenKind::Keyword,
        "comment" => TokenKind::Comment,
        "string" | "regexp" => TokenKind::String,
        "number" => TokenKind::Number,
        "operator" => TokenKind::Operator,
        _ => TokenKind::Variable,
    }
}

/// Map tree-sitter capture name → TokenKind
pub fn from_capture(name: &str) -> Option<TokenKind> {
    // strip dotted suffix priority: take full match first
    let base = name.split('.').next().unwrap_or(name);
    Some(match name {
        "keyword" | "keyword.function" | "keyword.operator" | "keyword.return" => {
            TokenKind::Keyword
        }
        "keyword.control" | "conditional" | "repeat" => TokenKind::KeywordControl,
        "keyword.import" | "include" => TokenKind::KeywordImport,
        "string" | "string.special" | "character" => TokenKind::String,
        "comment" | "comment.documentation" | "comment.line" | "comment.block" => {
            TokenKind::Comment
        }
        "number" | "float" | "boolean" => TokenKind::Number,
        "type" | "type.builtin" | "type.definition" | "constructor" => TokenKind::TypeName,
        "function" | "function.call" | "function.builtin" => TokenKind::Function,
        "function.method" | "method" | "method.call" => TokenKind::Method,
        "function.macro" | "macro" => TokenKind::Macro,
        "namespace" | "module" => TokenKind::Namespace,
        "variable.parameter" | "parameter" => TokenKind::Parameter,
        "property" | "field" | "variable.member" => TokenKind::Property,
        "constant" | "constant.builtin" => TokenKind::Constant,
        "variable" | "variable.builtin" => TokenKind::Variable,
        "operator" => TokenKind::Operator,
        "punctuation" | "punctuation.bracket" | "punctuation.delimiter" => TokenKind::Punctuation,
        "attribute" | "annotation" => TokenKind::Attribute,
        "lifetime" => TokenKind::Lifetime,
        // fallback by base
        _ => match base {
            "keyword" => TokenKind::Keyword,
            "string" => TokenKind::String,
            "comment" => TokenKind::Comment,
            "number" => TokenKind::Number,
            "type" => TokenKind::TypeName,
            "function" => TokenKind::Function,
            "method" => TokenKind::Method,
            "macro" => TokenKind::Macro,
            "namespace" | "module" => TokenKind::Namespace,
            "parameter" => TokenKind::Parameter,
            "property" | "field" => TokenKind::Property,
            "constant" => TokenKind::Constant,
            "variable" => TokenKind::Variable,
            "operator" => TokenKind::Operator,
            "punctuation" => TokenKind::Punctuation,
            "attribute" => TokenKind::Attribute,
            "constructor" => TokenKind::TypeName,
            _ => return None,
        },
    })
}

// ── Fallback line tokenizer ─────────────────────────────

pub fn highlight_line(line: &str, ext: Option<&str>) -> Vec<(TokenKind, usize, usize)> {
    let rules = match ext {
        Some("rs") => rust_rules(),
        Some("py" | "pyi") => py_rules(),
        Some("ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs") => js_rules(),
        Some("go") => go_rules(),
        Some("c" | "h") => c_rules(),
        Some("cpp" | "hpp" | "cc" | "cxx" | "hh" | "hxx") => cpp_rules(),
        Some("sh" | "bash" | "zsh") => shell_rules(),
        Some("html" | "htm") => html_rules(),
        Some("css" | "scss" | "less") => css_rules(),
        Some("toml") => toml_rules(),
        Some("yaml" | "yml") => yaml_rules(),
        Some("sql") => sql_rules(),
        Some("md" | "mdx") => md_rules(),
        Some("json" | "jsonc") => json_rules(),
        Some("java" | "kt" | "kts") => java_rules(),
        Some("lua") => lua_rules(),
        Some("rb") => rb_rules(),
        Some("swift") => swift_rules(),
        Some("zig") => zig_rules(),
        Some("dart") => dart_rules(),
        Some("php") => php_rules(),
        Some("cs") => csharp_rules(),
        Some("scala") => scala_rules(),
        Some("hs") => haskell_rules(),
        Some("ex" | "exs") => elixir_rules(),
        Some("nim") => nim_rules(),
        Some("vue" | "svelte") => js_rules(),
        _ => generic_rules(),
    };
    tokenize(line, &rules, ext)
}

struct LangRules {
    line_comment: Option<&'static str>,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    imports: &'static [&'static str],
    controls: &'static [&'static str],
}

fn rust_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "fn", "let", "mut", "struct", "impl", "enum", "trait", "pub", "use", "mod",
            "const", "static", "type", "where", "unsafe", "async", "await", "move", "ref",
            "self", "Self", "super", "crate", "extern", "dyn", "as", "in", "box",
        ],
        types: &[
            "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128",
            "usize", "f32", "f64", "bool", "char", "str", "String", "Vec", "Option", "Result",
            "Box", "Rc", "Arc", "HashMap", "HashSet", "Ok", "Err", "Some", "None",
        ],
        imports: &["use", "mod", "extern", "crate"],
        controls: &[
            "if", "else", "match", "while", "for", "loop", "return", "break", "continue",
            "yield",
        ],
    }
}

fn py_rules() -> LangRules {
    LangRules {
        line_comment: Some("#"),
        keywords: &[
            "def", "class", "lambda", "with", "as", "pass", "global", "nonlocal", "del",
            "assert", "yield", "from", "import", "True", "False", "None", "and", "or", "not",
            "is", "in", "async", "await",
        ],
        types: &["int", "float", "str", "bool", "list", "dict", "set", "tuple", "object"],
        imports: &["import", "from", "as"],
        controls: &[
            "if", "elif", "else", "for", "while", "try", "except", "finally", "raise",
            "return", "break", "continue", "with",
        ],
    }
}

fn js_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "function", "const", "let", "var", "class", "interface", "type", "enum",
            "import", "export", "default", "async", "await", "new", "this", "super",
            "extends", "implements", "typeof", "instanceof", "void", "delete", "of",
            "from", "as", "in", "true", "false", "null", "undefined",
        ],
        types: &[
            "string", "number", "boolean", "any", "unknown", "never", "void", "object",
            "Promise", "Array", "Record", "Map", "Set",
        ],
        imports: &["import", "export", "from", "require"],
        controls: &[
            "if", "else", "for", "while", "switch", "case", "try", "catch", "finally",
            "throw", "return", "break", "continue", "yield",
        ],
    }
}

fn go_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "func", "var", "const", "type", "struct", "interface", "map", "chan", "go",
            "defer", "select", "package", "import", "range", "true", "false", "nil", "iota",
        ],
        types: &[
            "string", "int", "int8", "int16", "int32", "int64", "uint", "byte", "rune",
            "float32", "float64", "bool", "error", "any",
        ],
        imports: &["import", "package"],
        controls: &[
            "if", "else", "for", "switch", "case", "select", "return", "break", "continue",
            "fallthrough", "goto",
        ],
    }
}

fn c_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "return", "if", "else", "for", "while", "do", "switch", "case", "break",
            "continue", "struct", "enum", "typedef", "sizeof", "static", "const", "void",
            "extern", "goto", "default", "union", "volatile", "register", "restrict",
        ],
        types: &[
            "int", "char", "float", "double", "long", "short", "unsigned", "signed",
            "size_t", "bool", "uint8_t", "uint16_t", "uint32_t", "uint64_t",
        ],
        imports: &["include"],
        controls: &["if", "else", "for", "while", "do", "switch", "case", "return", "break", "continue", "goto"],
    }
}

fn cpp_rules() -> LangRules {
    let mut _k = c_rules();
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "class", "namespace", "template", "typename", "public", "private", "protected",
            "virtual", "override", "final", "new", "delete", "this", "using", "constexpr",
            "noexcept", "nullptr", "auto", "concept", "requires", "co_await", "co_yield",
            "true", "false", "return", "if", "else", "for", "while", "switch", "case",
            "struct", "enum", "typedef", "static", "const", "void",
        ],
        types: c_rules().types,
        imports: &["include", "using", "namespace"],
        controls: c_rules().controls,
    }
}

fn shell_rules() -> LangRules {
    LangRules {
        line_comment: Some("#"),
        keywords: &[
            "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case",
            "esac", "function", "return", "in", "select", "time", "coproc",
        ],
        types: &[],
        imports: &["source"],
        controls: &["if", "then", "else", "elif", "fi", "for", "while", "do", "done", "return"],
    }
}

fn html_rules() -> LangRules {
    LangRules {
        line_comment: None,
        keywords: &[],
        types: &[],
        imports: &[],
        controls: &[],
    }
}

fn css_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &["important", "from", "to"],
        types: &[],
        imports: &["import"],
        controls: &[],
    }
}

fn toml_rules() -> LangRules {
    LangRules {
        line_comment: Some("#"),
        keywords: &["true", "false"],
        types: &[],
        imports: &[],
        controls: &[],
    }
}

fn yaml_rules() -> LangRules {
    LangRules {
        line_comment: Some("#"),
        keywords: &["true", "false", "null", "yes", "no"],
        types: &[],
        imports: &[],
        controls: &[],
    }
}

fn sql_rules() -> LangRules {
    LangRules {
        line_comment: Some("--"),
        keywords: &[
            "SELECT", "FROM", "WHERE", "INSERT", "UPDATE", "DELETE", "JOIN", "LEFT",
            "RIGHT", "INNER", "OUTER", "ON", "AS", "AND", "OR", "NOT", "NULL", "CREATE",
            "TABLE", "INDEX", "VALUES", "INTO", "SET", "ORDER", "BY", "GROUP", "HAVING",
            "LIMIT", "OFFSET", "select", "from", "where", "insert", "update", "delete",
        ],
        types: &["INT", "TEXT", "VARCHAR", "BOOLEAN", "TIMESTAMP", "JSON"],
        imports: &[],
        controls: &[],
    }
}

fn md_rules() -> LangRules {
    LangRules {
        line_comment: None,
        keywords: &[],
        types: &[],
        imports: &[],
        controls: &[],
    }
}

fn json_rules() -> LangRules {
    LangRules {
        line_comment: None,
        keywords: &["true", "false", "null"],
        types: &[],
        imports: &[],
        controls: &[],
    }
}

fn java_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "class", "interface", "enum", "extends", "implements", "public", "private",
            "protected", "static", "final", "abstract", "void", "new", "this", "super",
            "import", "package", "true", "false", "null", "var", "record",
        ],
        types: &["int", "long", "String", "boolean", "Object", "List", "Map"],
        imports: &["import", "package"],
        controls: &["if", "else", "for", "while", "switch", "case", "try", "catch", "return", "break", "continue", "throw"],
    }
}

fn lua_rules() -> LangRules {
    LangRules {
        line_comment: Some("--"),
        keywords: &[
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function",
            "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then",
            "true", "until", "while",
        ],
        types: &[],
        imports: &["require"],
        controls: &["if", "else", "elseif", "for", "while", "repeat", "return", "break"],
    }
}

fn rb_rules() -> LangRules {
    LangRules {
        line_comment: Some("#"),
        keywords: &[
            "def", "class", "module", "end", "do", "begin", "rescue", "ensure", "yield",
            "super", "self", "true", "false", "nil", "and", "or", "not", "require",
        ],
        types: &[],
        imports: &["require", "include", "extend"],
        controls: &["if", "elsif", "else", "unless", "case", "when", "while", "until", "for", "return", "break", "next"],
    }
}

fn swift_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "func", "var", "let", "class", "struct", "enum", "protocol", "extension",
            "import", "guard", "defer", "async", "await", "true", "false", "nil", "self",
            "Self", "public", "private", "static",
        ],
        types: &["Int", "Double", "String", "Bool", "Array", "Dictionary", "Optional"],
        imports: &["import"],
        controls: &["if", "else", "switch", "case", "for", "while", "return", "guard", "throw", "try", "catch"],
    }
}

fn zig_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "const", "var", "fn", "pub", "struct", "enum", "union", "try", "catch",
            "defer", "errdefer", "async", "await", "export", "extern", "inline",
            "comptime", "test", "true", "false", "null", "undefined", "and", "or",
        ],
        types: &["i8", "i16", "i32", "i64", "u8", "u32", "u64", "f32", "f64", "bool", "void", "usize"],
        imports: &["@import"],
        controls: &["if", "else", "while", "for", "return", "break", "continue", "switch"],
    }
}

fn dart_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "class", "extends", "implements", "with", "mixin", "import", "export",
            "void", "var", "final", "const", "async", "await", "true", "false", "null",
            "this", "super", "static", "abstract", "get", "set",
        ],
        types: &["int", "double", "num", "String", "bool", "List", "Map", "Set"],
        imports: &["import", "export"],
        controls: &["if", "else", "for", "while", "switch", "case", "try", "catch", "return", "throw", "yield"],
    }
}

fn php_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "function", "class", "interface", "trait", "extends", "implements", "public",
            "private", "protected", "static", "namespace", "use", "as", "new", "echo",
            "true", "false", "null",
        ],
        types: &["int", "float", "string", "bool", "array", "object", "void"],
        imports: &["use", "namespace", "require", "include"],
        controls: &["if", "else", "elseif", "foreach", "for", "while", "switch", "case", "try", "catch", "return", "throw"],
    }
}

fn csharp_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "class", "struct", "interface", "enum", "namespace", "using", "public",
            "private", "protected", "static", "void", "async", "await", "var", "new",
            "this", "base", "true", "false", "null", "get", "set",
        ],
        types: &["int", "string", "bool", "object", "List", "Dictionary", "Task"],
        imports: &["using", "namespace"],
        controls: &["if", "else", "for", "foreach", "while", "switch", "case", "try", "catch", "return", "break", "continue", "throw", "yield"],
    }
}

fn scala_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "def", "val", "var", "class", "object", "trait", "extends", "with", "import",
            "package", "true", "false", "null", "new", "this", "super", "implicit",
            "given", "using", "enum", "case",
        ],
        types: &["Int", "String", "Boolean", "Unit", "List", "Map", "Option"],
        imports: &["import", "package"],
        controls: &["if", "else", "for", "while", "match", "return", "throw", "try", "catch", "yield"],
    }
}

fn haskell_rules() -> LangRules {
    LangRules {
        line_comment: Some("--"),
        keywords: &[
            "module", "import", "where", "let", "in", "if", "then", "else", "case", "of",
            "data", "type", "newtype", "class", "instance", "deriving", "do", "qualified",
            "as", "hiding",
        ],
        types: &["Int", "Integer", "String", "Bool", "Maybe", "Either", "IO"],
        imports: &["import", "module"],
        controls: &["if", "then", "else", "case", "of", "do"],
    }
}

fn elixir_rules() -> LangRules {
    LangRules {
        line_comment: Some("#"),
        keywords: &[
            "def", "defp", "defmodule", "defstruct", "defprotocol", "defimpl", "do", "end",
            "fn", "true", "false", "nil", "when", "case", "cond", "with", "for", "if",
            "unless", "try", "rescue", "catch", "after", "import", "alias", "require", "use",
        ],
        types: &[],
        imports: &["import", "alias", "require", "use"],
        controls: &["if", "unless", "case", "cond", "with", "for", "try", "rescue", "catch"],
    }
}

fn nim_rules() -> LangRules {
    LangRules {
        line_comment: Some("#"),
        keywords: &[
            "proc", "func", "method", "template", "macro", "var", "let", "const", "type",
            "object", "enum", "import", "from", "as", "export", "true", "false", "nil",
        ],
        types: &["int", "string", "bool", "float", "seq", "array"],
        imports: &["import", "from", "include"],
        controls: &["if", "elif", "else", "for", "while", "case", "of", "return", "break", "continue", "try", "except"],
    }
}

fn generic_rules() -> LangRules {
    LangRules {
        line_comment: Some("//"),
        keywords: &[
            "if", "else", "for", "while", "return", "function", "fn", "def", "class",
            "struct", "const", "let", "var", "true", "false", "null", "nil", "import",
        ],
        types: &[],
        imports: &["import", "include", "use", "from"],
        controls: &["if", "else", "for", "while", "return", "break", "continue", "switch", "case"],
    }
}

fn tokenize(line: &str, rules: &LangRules, ext: Option<&str>) -> Vec<(TokenKind, usize, usize)> {
    let mut tokens: Vec<(TokenKind, usize, usize)> = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;

    // line comment
    if let Some(comment_prefix) = rules.line_comment {
        if let Some(pos) = find_not_in_string(&chars, comment_prefix) {
            if pos < len {
                tokens.push((TokenKind::Comment, pos, len));
                if pos == 0 {
                    return tokens;
                }
            }
        }
    }

    // rust attributes
    if matches!(ext, Some("rs")) && line.trim_start().starts_with("#[") {
        if let Some(start) = line.find("#[") {
            if let Some(end_rel) = line[start..].find(']') {
                tokens.push((TokenKind::Attribute, start, start + end_rel + 1));
            }
        }
    }

    while i < len {
        // strings
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            let start = i;
            i += 1;
            while i < len && chars[i] != quote {
                if chars[i] == '\\' && i + 1 < len {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < len {
                i += 1;
            }
            tokens.push((TokenKind::String, start, i));
            continue;
        }

        // raw string-ish rust r#"
        if matches!(ext, Some("rs")) && chars[i] == 'r' && i + 1 < len && (chars[i + 1] == '"' || chars[i + 1] == '#') {
            // fall through to identifier / simple string after r
        }

        // numbers
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < len
                && (chars[i].is_ascii_alphanumeric()
                    || chars[i] == '.'
                    || chars[i] == '_'
                    || chars[i] == 'x'
                    || chars[i] == 'X')
            {
                i += 1;
            }
            tokens.push((TokenKind::Number, start, i));
            continue;
        }

        // identifiers / keywords / calls / macros
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '!') {
                i += 1;
            }
            // strip trailing ! for matching keyword, then detect macro
            let raw: String = chars[start..i].iter().collect();
            let is_macro = raw.ends_with('!');
            let word = raw.trim_end_matches('!');

            let mut kind = if rules.imports.contains(&word) {
                TokenKind::KeywordImport
            } else if rules.controls.contains(&word) {
                TokenKind::KeywordControl
            } else if rules.keywords.contains(&word) {
                TokenKind::Keyword
            } else if rules.types.contains(&word)
                || (word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
                    && word.chars().all(|c| c.is_alphanumeric() || c == '_'))
            {
                TokenKind::TypeName
            } else if word.chars().all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '_')
                && word.len() > 1
            {
                TokenKind::Constant
            } else {
                TokenKind::Variable
            };

            if is_macro {
                kind = TokenKind::Macro;
            } else {
                // function call: ident(
                let mut j = i;
                while j < len && chars[j].is_whitespace() {
                    j += 1;
                }
                if j < len && chars[j] == '(' && !matches!(
                    kind,
                    TokenKind::Keyword
                        | TokenKind::KeywordControl
                        | TokenKind::KeywordImport
                        | TokenKind::TypeName
                ) {
                    kind = TokenKind::Function;
                }
            }

            if !matches!(kind, TokenKind::Variable) || is_macro {
                tokens.push((kind, start, i));
            } else if kind == TokenKind::Variable {
                // still skip plain variables to keep noise down — types/fns already tagged
            }
            continue;
        }

        // operators
        if matches!(
            chars[i],
            '+' | '-' | '*' | '/' | '%' | '=' | '!' | '<' | '>' | '&' | '|' | '^' | '~' | '?'
        ) {
            let start = i;
            i += 1;
            while i < len
                && matches!(
                    chars[i],
                    '+' | '-' | '*' | '/' | '%' | '=' | '!' | '<' | '>' | '&' | '|' | '^' | '?'
                )
            {
                i += 1;
            }
            tokens.push((TokenKind::Operator, start, i));
            continue;
        }

        if matches!(chars[i], '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '.') {
            tokens.push((TokenKind::Punctuation, i, i + 1));
            i += 1;
            continue;
        }

        i += 1;
    }

    tokens
}

fn find_not_in_string(chars: &[char], needle: &str) -> Option<usize> {
    let n: Vec<char> = needle.chars().collect();
    let n_len = n.len();
    let mut i = 0;
    let mut in_string = false;
    let mut string_char = '"';

    while i + n_len <= chars.len() {
        let c = chars[i];
        if (c == '"' || c == '\'') && (i == 0 || chars[i - 1] != '\\') {
            if !in_string {
                in_string = true;
                string_char = c;
            } else if c == string_char {
                in_string = false;
            }
        }
        if !in_string && chars[i..i + n_len] == n[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_rust_fn_and_string() {
        let t = highlight_line(r#"fn main() { println!("hi"); }"#, Some("rs"));
        assert!(t.iter().any(|(k, _, _)| matches!(k, TokenKind::Keyword | TokenKind::KeywordControl)));
        assert!(t.iter().any(|(k, _, _)| matches!(k, TokenKind::String)));
        assert!(t.iter().any(|(k, _, _)| matches!(k, TokenKind::Macro | TokenKind::Function)));
    }

    #[test]
    fn capture_mapping() {
        assert_eq!(from_capture("function.macro"), Some(TokenKind::Macro));
        assert_eq!(from_capture("type.builtin"), Some(TokenKind::TypeName));
    }
}
