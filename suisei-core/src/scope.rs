//! Lexical scope resolution: which symbols are visible at a cursor position.
//!
//! Completion until now offered language KEYWORDS and nothing else — no symbol
//! declared in the buffer ever appeared. The requirement is not to filter an
//! over-eager list but to build a correct one: a `let` or nested `fn` inside
//! function A must not be offered while editing function C, while a top-level
//! item must be offered everywhere.
//!
//! The rule is ordinary lexical scoping, read off the tree-sitter parse the
//! editor already keeps warm for highlighting:
//!
//! 1. find the node containing the cursor,
//! 2. walk UP to the root, collecting the chain of enclosing scopes,
//! 3. from each scope in that chain take the declarations made *directly* in
//!    it, never descending into a scope that does not contain the cursor.
//!
//! Step 3 is what excludes function A from function C: A's body is a scope that
//! is not on C's chain, so nothing inside it is ever reached. A declaration's
//! NAME, however, belongs to the scope that encloses it — so `fn helper()` at
//! file level is visible everywhere even though its body is not.
//!
//! Only the grammars Suisei already links are handled. JS and TS ship a
//! `LOCALS_QUERY` and the rest do not, so this walks node kinds rather than
//! depending on queries that mostly do not exist.

use tree_sitter::{Node, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Variable,
    Parameter,
    Type,
    Constant,
    Module,
}

impl SymbolKind {
    /// Short label for the completion popup's detail column.
    pub fn detail(self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Variable => "let",
            SymbolKind::Parameter => "param",
            SymbolKind::Type => "type",
            SymbolKind::Constant => "const",
            SymbolKind::Module => "mod",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSymbol {
    pub name: String,
    pub kind: SymbolKind,
    /// Declared at file top level, so visible from anywhere in the file.
    pub global: bool,
    /// How many scopes out from the cursor it was found. 0 = the innermost
    /// scope. Used only for ordering: nearer bindings should be offered first,
    /// which is also what shadowing implies.
    pub depth: usize,
}

/// Languages this module knows how to walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeLang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    C,
    Go,
}

impl ScopeLang {
    pub fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(ScopeLang::Rust),
            "py" => Some(ScopeLang::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(ScopeLang::JavaScript),
            "ts" | "tsx" => Some(ScopeLang::TypeScript),
            "c" | "h" => Some(ScopeLang::C),
            "go" => Some(ScopeLang::Go),
            _ => None,
        }
    }

    /// Node kinds that introduce a new binding scope.
    ///
    /// A function's *body* is the scope, not the function node — parameters are
    /// handled separately so they land inside the body's scope rather than the
    /// one that encloses the function.
    fn is_scope(self, kind: &str) -> bool {
        match self {
            ScopeLang::Rust => matches!(
                kind,
                "block" | "function_item" | "closure_expression" | "mod_item" | "impl_item"
            ),
            ScopeLang::Python => matches!(kind, "function_definition" | "class_definition"),
            ScopeLang::JavaScript | ScopeLang::TypeScript => matches!(
                kind,
                "statement_block"
                    | "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "method_definition"
                    | "class_body"
            ),
            ScopeLang::C => matches!(kind, "compound_statement" | "function_definition"),
            ScopeLang::Go => matches!(
                kind,
                "block" | "function_declaration" | "method_declaration" | "func_literal"
            ),
        }
    }
}

/// Symbols visible at `byte`, nearest scope first, de-duplicated by name.
///
/// Shadowing falls out of the ordering: an inner binding is reached first, and
/// the outer one with the same name is dropped as a duplicate.
pub fn visible_at(tree: &Tree, src: &str, byte: usize, lang: ScopeLang) -> Vec<ScopeSymbol> {
    let root = tree.root_node();
    let byte = byte.min(src.len());
    // The node the cursor sits in. Falls back to the root for an empty file or
    // a position past the last node.
    let start = root
        .descendant_for_byte_range(byte, byte)
        .unwrap_or(root);

    // Chain of enclosing scopes, innermost first, always ending at the root.
    let mut chain: Vec<Node> = Vec::new();
    let mut cur = Some(start);
    while let Some(n) = cur {
        if lang.is_scope(n.kind()) || n.id() == root.id() {
            chain.push(n);
        }
        cur = n.parent();
    }
    if chain.last().map(|n| n.id()) != Some(root.id()) {
        chain.push(root);
    }

    let mut out: Vec<ScopeSymbol> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let last = chain.len().saturating_sub(1);
    for (depth, scope) in chain.iter().enumerate() {
        let global = depth == last;
        let mut found = Vec::new();
        collect_in_scope(*scope, src, lang, *scope, &mut found);
        for (name, kind) in found {
            if name.is_empty() || seen.iter().any(|s| s == &name) {
                continue;
            }
            seen.push(name.clone());
            out.push(ScopeSymbol {
                name,
                kind,
                global,
                depth,
            });
        }
    }
    out
}

/// Declarations made directly inside `scope`, without entering a nested scope.
///
/// `node` walks down; `scope` is the boundary. Descending stops at any node
/// that is itself a scope (other than the boundary), which is precisely what
/// keeps one function's locals out of another's completion list.
fn collect_in_scope(
    node: Node,
    src: &str,
    lang: ScopeLang,
    scope: Node,
    out: &mut Vec<(String, SymbolKind)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        declarations(child, src, lang, out);
        // A nested scope's *contents* belong to that scope, not this one. Its
        // name was already taken above by `declarations`.
        //
        // Exception: when the boundary is the function node itself, its body is
        // where the cursor lives, so keep descending into it. Parameters are
        // read the same way, which is why they land beside the body's locals
        // instead of leaking outward.
        let nested = lang.is_scope(child.kind()) && child.id() != scope.id();
        if nested && !contains_scope_boundary(child, scope) {
            continue;
        }
        collect_in_scope(child, src, lang, scope, out);
    }
}

/// Is `scope` inside `node`? Used to keep descending toward the cursor's own
/// body rather than stopping at the function that owns it.
fn contains_scope_boundary(node: Node, scope: Node) -> bool {
    let mut cur = Some(scope);
    while let Some(n) = cur {
        if n.id() == node.id() {
            return true;
        }
        cur = n.parent();
    }
    false
}

/// Names this node declares, if any.
fn declarations(node: Node, src: &str, lang: ScopeLang, out: &mut Vec<(String, SymbolKind)>) {
    fn text(n: Node, src: &str) -> String {
        n.utf8_text(src.as_bytes()).unwrap_or("").to_string()
    }
    fn named<'t>(n: Node<'t>, field: &str) -> Option<Node<'t>> {
        n.child_by_field_name(field)
    }
    let text = |n: Node| -> String { text(n, src) };

    match lang {
        ScopeLang::Rust => match node.kind() {
            "function_item" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Function));
                }
            }
            "let_declaration" => {
                if let Some(p) = named(node, "pattern") {
                    pattern_names(p, src, SymbolKind::Variable, out);
                }
            }
            "parameter" => {
                if let Some(p) = named(node, "pattern") {
                    pattern_names(p, src, SymbolKind::Parameter, out);
                }
            }
            "const_item" | "static_item" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Constant));
                }
            }
            "struct_item" | "enum_item" | "trait_item" | "type_item" | "union_item" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Type));
                }
            }
            "mod_item" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Module));
                }
            }
            _ => {}
        },
        ScopeLang::Python => match node.kind() {
            "function_definition" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Function));
                }
            }
            "class_definition" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Type));
                }
            }
            "assignment" => {
                if let Some(l) = named(node, "left") {
                    pattern_names(l, src, SymbolKind::Variable, out);
                }
            }
            "identifier" => {
                // Parameters: `parameters` holds bare identifiers.
                if node
                    .parent()
                    .is_some_and(|p| p.kind() == "parameters" || p.kind() == "lambda_parameters")
                {
                    out.push((text(node), SymbolKind::Parameter));
                }
            }
            _ => {}
        },
        ScopeLang::JavaScript | ScopeLang::TypeScript => match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Function));
                }
            }
            "class_declaration" | "interface_declaration" | "type_alias_declaration" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Type));
                }
            }
            "variable_declarator" => {
                if let Some(n) = named(node, "name") {
                    pattern_names(n, src, SymbolKind::Variable, out);
                }
            }
            "required_parameter" | "optional_parameter" => {
                if let Some(p) = named(node, "pattern") {
                    pattern_names(p, src, SymbolKind::Parameter, out);
                }
            }
            "identifier" => {
                if node.parent().is_some_and(|p| p.kind() == "formal_parameters") {
                    out.push((text(node), SymbolKind::Parameter));
                }
            }
            _ => {}
        },
        ScopeLang::C => match node.kind() {
            "function_definition" => {
                if let Some(d) = named(node, "declarator") {
                    if let Some(n) = innermost_declarator_name(d, src) {
                        out.push((n, SymbolKind::Function));
                    }
                }
            }
            "declaration" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind().ends_with("declarator") || child.kind() == "identifier" {
                        if let Some(n) = innermost_declarator_name(child, src) {
                            out.push((n, SymbolKind::Variable));
                        }
                    }
                }
            }
            "parameter_declaration" => {
                if let Some(d) = named(node, "declarator") {
                    if let Some(n) = innermost_declarator_name(d, src) {
                        out.push((n, SymbolKind::Parameter));
                    }
                }
            }
            _ => {}
        },
        ScopeLang::Go => match node.kind() {
            "function_declaration" | "method_declaration" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Function));
                }
            }
            "type_spec" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Type));
                }
            }
            "short_var_declaration" => {
                if let Some(l) = named(node, "left") {
                    pattern_names(l, src, SymbolKind::Variable, out);
                }
            }
            "var_spec" | "const_spec" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Variable));
                }
            }
            "parameter_declaration" => {
                if let Some(n) = named(node, "name") {
                    out.push((text(n), SymbolKind::Parameter));
                }
            }
            _ => {}
        },
    }
}

/// Every identifier bound by a pattern — `let (a, b)` binds both.
fn pattern_names(node: Node, src: &str, kind: SymbolKind, out: &mut Vec<(String, SymbolKind)>) {
    if node.kind() == "identifier" || node.kind() == "shorthand_property_identifier_pattern" {
        if let Ok(t) = node.utf8_text(src.as_bytes()) {
            out.push((t.to_string(), kind));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        pattern_names(child, src, kind, out);
    }
}

/// C declarators nest: `*foo[3]` -> the identifier at the centre.
fn innermost_declarator_name(node: Node, src: &str) -> Option<String> {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return node.utf8_text(src.as_bytes()).ok().map(str::to_string);
    }
    if let Some(d) = node.child_by_field_name("declarator") {
        return innermost_declarator_name(d, src);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(n) = innermost_declarator_name(child, src) {
            return Some(n);
        }
    }
    None
}
