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
//! ## Why the languages are data
//!
//! Each language used to be a hand-written `match` arm, and at six languages
//! that read fine. At thirteen it stops being reviewable: every arm repeats the
//! same four shapes — *take this node's `name` field*, *walk this pattern for
//! identifiers*, *unwrap a C declarator*, *this bare identifier is a parameter
//! because of its parent*. So the shapes are [`Bind`] and each language is a
//! table of [`DeclRule`]. Adding a language is one slice plus one list of scope
//! node kinds, and a reviewer can check it against the grammar's node-types
//! without reading any control flow.
//!
//! Which extension is which language is NOT decided here — see [`crate::lang`].
//! Two tables answering that question is how C++ came to highlight correctly
//! and silently offer no symbols at all.

use tree_sitter::{Node, Tree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    /// A function declared in a type's body — `impl`, `class`, `trait`,
    /// `interface`. Split out from `Function` because the completion popup
    /// showing `fn` for everything was the specific complaint: `new` is a
    /// function and `is_bright` is a method, and the list should say so.
    Method,
    Variable,
    Parameter,
    Type,
    Constant,
    Module,
}

impl SymbolKind {
    /// Short label for the completion popup's detail column, used when the
    /// declaration carries no type annotation to show instead.
    pub fn detail(self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Method => "method",
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
    /// The declared type, verbatim from the source, when the grammar has one to
    /// give — `i32`, `String`, `number`. Absent in languages that mostly do not
    /// annotate (Python, JavaScript, Ruby, Lua), which is why the popup falls
    /// back to [`SymbolKind::detail`] rather than showing a blank column.
    pub ty: Option<String>,
    /// Declared at file top level, so visible from anywhere in the file.
    pub global: bool,
    /// How many scopes out from the cursor it was found. 0 = the innermost
    /// scope. Used only for ordering: nearer bindings should be offered first,
    /// which is also what shadowing implies.
    pub depth: usize,
}

impl ScopeSymbol {
    /// What the completion popup shows beside the name: the type when there is
    /// one, otherwise what kind of thing this is.
    pub fn detail(&self) -> String {
        match &self.ty {
            Some(t) => t.clone(),
            None => self.kind.detail().to_string(),
        }
    }
}

/// Languages this module knows how to walk.
///
/// A language having a grammar does not put it here — walking a tree for
/// bindings is separate work with its own conformance rows in
/// `tests/scope_language_conformance.rs`. See [`crate::lang::Lang::scope`] for
/// which grammars do and do not map onto one, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeLang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    C,
    Cpp,
    Go,
    Java,
    CSharp,
    Ruby,
    Lua,
    Swift,
    Php,
    Zig,
    Dart,
    ObjC,
}

/// Where a rule finds the name (or names) a node declares.
#[derive(Debug, Clone, Copy)]
enum Bind {
    /// `child_by_field_name(f)` is the identifier.
    Field(&'static str),
    /// `child_by_field_name(f)` is a PATTERN — `let (a, b)` binds both, so every
    /// identifier under it is taken.
    Pattern(&'static str),
    /// C-family: `child_by_field_name(f)` is a declarator that nests —
    /// `*foo[3]` — so unwrap to the identifier at the centre.
    Declarator(&'static str),
    /// C-family `declaration`, which holds one or more declarators as direct
    /// children rather than behind a field.
    DeclaratorChildren,
    /// The node IS the identifier, and counts only because of its parent —
    /// how several grammars spell a bare parameter.
    SelfWhenParent(&'static [&'static str]),
    /// The first direct child that is a bare identifier. Zig's
    /// `variable_declaration` and Objective-C's `class_interface` put the name
    /// there with no field name to ask for, and walking the whole subtree for
    /// identifiers would bind the value as well as the variable.
    FirstIdentifierChild,
    /// The nearest `name` field anywhere above the body. Dart buries it:
    /// `function_declaration → signature → function_signature → name`, two
    /// levels down and one deeper again for a method. The search stops at any
    /// node that is itself a scope, so it can never reach into the body and
    /// come back with a local's name instead.
    BuriedName,
}

/// Where a rule finds the declared type, if the grammar records one.
#[derive(Debug, Clone, Copy)]
enum Ty {
    None,
    /// A field on the declaring node itself.
    Own(&'static str),
    /// A field on the PARENT — Java and C# put the type on the declaration and
    /// the name on a `variable_declarator` beneath it.
    Parent(&'static str),
}

struct DeclRule {
    kind: &'static str,
    bind: Bind,
    sym: SymbolKind,
    ty: Ty,
}

/// Shorthand for the overwhelmingly common rule: a node with a `name` field.
///
/// A macro rather than a `const fn` so the tables below stay plain array
/// literals — those are promoted to `'static`, while an array built by calling
/// a function is a temporary and cannot be returned by reference.
macro_rules! named {
    ($kind:expr, $sym:expr) => {
        DeclRule {
            kind: $kind,
            bind: Bind::Field("name"),
            sym: $sym,
            ty: Ty::None,
        }
    };
}

/// The same, plus a type annotation on the node's own `type` field.
macro_rules! named_typed {
    ($kind:expr, $sym:expr) => {
        DeclRule {
            kind: $kind,
            bind: Bind::Field("name"),
            sym: $sym,
            ty: Ty::Own("type"),
        }
    };
}

impl ScopeLang {
    /// Which language an extension is — delegated, deliberately.
    ///
    /// This used to be a second extension table, and it had already drifted
    /// from the one in `syntax.rs`: C++ parsed and then offered nothing,
    /// because `syntax.rs` knew `cpp`/`hpp`/`cc`/`cxx`/`hh`/`hxx` and this did
    /// not. `pyi`, `mts` and `cts` were missing the same way. One table now,
    /// in `crate::lang`.
    pub fn from_ext(ext: &str) -> Option<Self> {
        crate::lang::Lang::from_ext(ext).and_then(crate::lang::Lang::scope)
    }

    /// Node kinds that introduce a new binding scope.
    ///
    /// A function's *body* is the scope, not the function node — parameters are
    /// handled separately so they land inside the body's scope rather than the
    /// one that encloses the function. A type's body is a scope too, which is
    /// what makes the functions inside it methods.
    fn scope_kinds(self) -> &'static [&'static str] {
        match self {
            ScopeLang::Rust => &[
                "block",
                "function_item",
                "closure_expression",
                "mod_item",
                "impl_item",
                "trait_item",
            ],
            ScopeLang::Python => &["function_definition", "class_definition", "lambda"],
            ScopeLang::JavaScript | ScopeLang::TypeScript => &[
                "statement_block",
                "function_declaration",
                "function_expression",
                "arrow_function",
                "method_definition",
                "class_body",
            ],
            ScopeLang::C => &["compound_statement", "function_definition"],
            ScopeLang::Cpp => &[
                "compound_statement",
                "function_definition",
                "namespace_definition",
                "class_specifier",
                "struct_specifier",
                "lambda_expression",
                "template_declaration",
            ],
            ScopeLang::Go => &[
                "block",
                "function_declaration",
                "method_declaration",
                "func_literal",
            ],
            ScopeLang::Java => &[
                "block",
                "class_body",
                "interface_body",
                "enum_body",
                "method_declaration",
                "constructor_declaration",
                "lambda_expression",
            ],
            ScopeLang::CSharp => &[
                "block",
                "declaration_list",
                "method_declaration",
                "constructor_declaration",
                "local_function_statement",
                "lambda_expression",
            ],
            ScopeLang::Ruby => &[
                "method",
                "singleton_method",
                "class",
                "singleton_class",
                "module",
                "block",
                "do_block",
                "lambda",
            ],
            ScopeLang::Lua => &[
                "block",
                "function_declaration",
                "function_definition",
                "for_statement",
            ],
            ScopeLang::Swift => &[
                "function_body",
                "statements",
                "class_body",
                "protocol_body",
                "function_declaration",
                "lambda_literal",
            ],
            ScopeLang::Php => &[
                "compound_statement",
                "function_definition",
                "method_declaration",
                "declaration_list",
                "anonymous_function_creation_expression",
                "namespace_definition",
            ],
            ScopeLang::Zig => &["block", "function_declaration"],
            // NOT `block`: Dart nests `function_body` around it, and the two
            // together are enough. `function_declaration` must be a scope so
            // its parameters land beside the body's locals rather than at file
            // level — which is also why its name is reached with `BuriedName`.
            ScopeLang::Dart => &[
                "block",
                "function_body",
                "class_body",
                "function_declaration",
                "method_declaration",
            ],
            // Objective-C's grammar is C's, with the `@`-declarations added.
            ScopeLang::ObjC => &[
                "compound_statement",
                "function_definition",
                "class_interface",
                "class_implementation",
            ],
        }
    }

    /// Node kinds whose direct functions are METHODS rather than free
    /// functions.
    ///
    /// Only needed where the grammar spells both the same way — Rust's
    /// `function_item` is a method purely because an `impl_item` encloses it.
    /// Grammars that already have a distinct node (`method_definition`,
    /// `method_declaration`) say so in their rule table instead.
    fn method_owner_kinds(self) -> &'static [&'static str] {
        match self {
            // Rust and Python are deliberately absent: a receiver, not an
            // enclosing type, is what makes a function a method there.
            // `Star::new` is declared inside `impl Star` and is still an
            // associated function — calling it a method is exactly the
            // imprecision the popup was asked to stop showing.
            ScopeLang::Swift => &["class_body", "protocol_body"],
            _ => &[],
        }
    }

    fn rules(self) -> &'static [DeclRule] {
        match self {
            ScopeLang::Rust => &[
                named!("function_item", SymbolKind::Function),
                DeclRule {
                    kind: "let_declaration",
                    bind: Bind::Pattern("pattern"),
                    sym: SymbolKind::Variable,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "parameter",
                    bind: Bind::Pattern("pattern"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                named_typed!("const_item", SymbolKind::Constant),
                named_typed!("static_item", SymbolKind::Constant),
                named!("struct_item", SymbolKind::Type),
                named!("enum_item", SymbolKind::Type),
                named!("trait_item", SymbolKind::Type),
                named!("type_item", SymbolKind::Type),
                named!("union_item", SymbolKind::Type),
                named!("mod_item", SymbolKind::Module),
            ],
            ScopeLang::Python => &[
                named!("function_definition", SymbolKind::Function),
                named!("class_definition", SymbolKind::Type),
                DeclRule {
                    kind: "assignment",
                    bind: Bind::Pattern("left"),
                    sym: SymbolKind::Variable,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "identifier",
                    bind: Bind::SelfWhenParent(&["parameters", "lambda_parameters"]),
                    sym: SymbolKind::Parameter,
                    ty: Ty::None,
                },
                DeclRule {
                    kind: "typed_parameter",
                    bind: Bind::Pattern("__self__"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                named!("default_parameter", SymbolKind::Parameter),
            ],
            ScopeLang::JavaScript | ScopeLang::TypeScript => &[
                named!("function_declaration", SymbolKind::Function),
                named!("generator_function_declaration", SymbolKind::Function),
                named!("method_definition", SymbolKind::Method),
                named!("class_declaration", SymbolKind::Type),
                named!("interface_declaration", SymbolKind::Type),
                named!("type_alias_declaration", SymbolKind::Type),
                named!("enum_declaration", SymbolKind::Type),
                DeclRule {
                    kind: "variable_declarator",
                    bind: Bind::Pattern("name"),
                    sym: SymbolKind::Variable,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "required_parameter",
                    bind: Bind::Pattern("pattern"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "optional_parameter",
                    bind: Bind::Pattern("pattern"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "identifier",
                    bind: Bind::SelfWhenParent(&["formal_parameters"]),
                    sym: SymbolKind::Parameter,
                    ty: Ty::None,
                },
            ],
            ScopeLang::C => &[
                DeclRule {
                    kind: "function_definition",
                    bind: Bind::Declarator("declarator"),
                    sym: SymbolKind::Function,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "declaration",
                    bind: Bind::DeclaratorChildren,
                    sym: SymbolKind::Variable,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "parameter_declaration",
                    bind: Bind::Declarator("declarator"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                named!("type_definition", SymbolKind::Type),
                named!("enum_specifier", SymbolKind::Type),
                named!("struct_specifier", SymbolKind::Type),
                named!("union_specifier", SymbolKind::Type),
            ],
            // C++ is C plus the constructs the C grammar has no spelling for.
            // Those are exactly what made "C++ is supported" untrue: a class,
            // a namespace or a template was invisible to both the highlighter
            // and this walk.
            ScopeLang::Cpp => &[
                DeclRule {
                    kind: "function_definition",
                    bind: Bind::Declarator("declarator"),
                    sym: SymbolKind::Function,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "declaration",
                    bind: Bind::DeclaratorChildren,
                    sym: SymbolKind::Variable,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "parameter_declaration",
                    bind: Bind::Declarator("declarator"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "optional_parameter_declaration",
                    bind: Bind::Declarator("declarator"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "field_declaration",
                    bind: Bind::DeclaratorChildren,
                    sym: SymbolKind::Variable,
                    ty: Ty::Own("type"),
                },
                named!("class_specifier", SymbolKind::Type),
                named!("struct_specifier", SymbolKind::Type),
                named!("union_specifier", SymbolKind::Type),
                named!("enum_specifier", SymbolKind::Type),
                named!("type_definition", SymbolKind::Type),
                named!("alias_declaration", SymbolKind::Type),
                named!("namespace_definition", SymbolKind::Module),
                DeclRule {
                    kind: "type_parameter_declaration",
                    bind: Bind::Pattern("__self__"),
                    sym: SymbolKind::Type,
                    ty: Ty::None,
                },
            ],
            ScopeLang::Go => &[
                named!("function_declaration", SymbolKind::Function),
                named!("method_declaration", SymbolKind::Method),
                named!("type_spec", SymbolKind::Type),
                DeclRule {
                    kind: "short_var_declaration",
                    bind: Bind::Pattern("left"),
                    sym: SymbolKind::Variable,
                    ty: Ty::None,
                },
                named_typed!("var_spec", SymbolKind::Variable),
                named_typed!("const_spec", SymbolKind::Constant),
                named_typed!("parameter_declaration", SymbolKind::Parameter),
            ],
            ScopeLang::Java => &[
                named_typed!("method_declaration", SymbolKind::Method),
                named!("constructor_declaration", SymbolKind::Method),
                named!("class_declaration", SymbolKind::Type),
                named!("interface_declaration", SymbolKind::Type),
                named!("enum_declaration", SymbolKind::Type),
                named!("record_declaration", SymbolKind::Type),
                named!("annotation_type_declaration", SymbolKind::Type),
                DeclRule {
                    kind: "variable_declarator",
                    bind: Bind::Field("name"),
                    sym: SymbolKind::Variable,
                    // Java puts the type on the enclosing declaration and the
                    // name on the declarator, so `int a = 1, b = 2` shares one
                    // type node between two names.
                    ty: Ty::Parent("type"),
                },
                named_typed!("formal_parameter", SymbolKind::Parameter),
                named!("catch_formal_parameter", SymbolKind::Parameter),
            ],
            ScopeLang::CSharp => &[
                named_typed!("method_declaration", SymbolKind::Method),
                named!("constructor_declaration", SymbolKind::Method),
                named_typed!("property_declaration", SymbolKind::Variable),
                named!("local_function_statement", SymbolKind::Function),
                named!("class_declaration", SymbolKind::Type),
                named!("struct_declaration", SymbolKind::Type),
                named!("interface_declaration", SymbolKind::Type),
                named!("record_declaration", SymbolKind::Type),
                named!("enum_declaration", SymbolKind::Type),
                named!("delegate_declaration", SymbolKind::Type),
                named!("namespace_declaration", SymbolKind::Module),
                DeclRule {
                    kind: "variable_declarator",
                    bind: Bind::Field("name"),
                    sym: SymbolKind::Variable,
                    ty: Ty::Parent("type"),
                },
                named_typed!("parameter", SymbolKind::Parameter),
            ],
            ScopeLang::Ruby => &[
                named!("method", SymbolKind::Method),
                named!("singleton_method", SymbolKind::Method),
                named!("class", SymbolKind::Type),
                named!("module", SymbolKind::Module),
                DeclRule {
                    kind: "assignment",
                    bind: Bind::Pattern("left"),
                    sym: SymbolKind::Variable,
                    ty: Ty::None,
                },
                DeclRule {
                    kind: "identifier",
                    bind: Bind::SelfWhenParent(&[
                        "method_parameters",
                        "block_parameters",
                        "lambda_parameters",
                    ]),
                    sym: SymbolKind::Parameter,
                    ty: Ty::None,
                },
                named!("optional_parameter", SymbolKind::Parameter),
                named!("keyword_parameter", SymbolKind::Parameter),
                named!("splat_parameter", SymbolKind::Parameter),
            ],
            ScopeLang::Lua => &[
                named!("function_declaration", SymbolKind::Function),
                DeclRule {
                    kind: "variable_declaration",
                    bind: Bind::Pattern("__self__"),
                    sym: SymbolKind::Variable,
                    ty: Ty::None,
                },
                DeclRule {
                    kind: "identifier",
                    bind: Bind::SelfWhenParent(&["parameters"]),
                    sym: SymbolKind::Parameter,
                    ty: Ty::None,
                },
            ],
            ScopeLang::Swift => &[
                named!("function_declaration", SymbolKind::Function),
                named!("class_declaration", SymbolKind::Type),
                named!("protocol_declaration", SymbolKind::Type),
                named!("typealias_declaration", SymbolKind::Type),
                DeclRule {
                    kind: "property_declaration",
                    bind: Bind::Pattern("name"),
                    sym: SymbolKind::Variable,
                    ty: Ty::None,
                },
                DeclRule {
                    kind: "parameter",
                    bind: Bind::Field("name"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
            ],
            ScopeLang::Php => &[
                named!("function_definition", SymbolKind::Function),
                named!("method_declaration", SymbolKind::Method),
                named!("class_declaration", SymbolKind::Type),
                named!("interface_declaration", SymbolKind::Type),
                named!("trait_declaration", SymbolKind::Type),
                named!("enum_declaration", SymbolKind::Type),
                named!("namespace_definition", SymbolKind::Module),
                DeclRule {
                    kind: "simple_parameter",
                    bind: Bind::Field("name"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "property_promotion_parameter",
                    bind: Bind::Field("name"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "assignment_expression",
                    bind: Bind::Pattern("left"),
                    sym: SymbolKind::Variable,
                    ty: Ty::None,
                },
            ],
            ScopeLang::Zig => &[
                named_typed!("function_declaration", SymbolKind::Function),
                // `const x: i32 = 1;` — the name is the first bare identifier,
                // with no field. Walking the subtree instead would bind the
                // initialiser's identifiers too.
                DeclRule {
                    kind: "variable_declaration",
                    bind: Bind::FirstIdentifierChild,
                    sym: SymbolKind::Variable,
                    ty: Ty::Own("type"),
                },
                named_typed!("parameter", SymbolKind::Parameter),
            ],
            ScopeLang::Dart => &[
                DeclRule {
                    kind: "function_declaration",
                    bind: Bind::BuriedName,
                    sym: SymbolKind::Function,
                    ty: Ty::None,
                },
                DeclRule {
                    kind: "method_declaration",
                    bind: Bind::BuriedName,
                    sym: SymbolKind::Method,
                    ty: Ty::None,
                },
                named!("class_declaration", SymbolKind::Type),
                named!("mixin_declaration", SymbolKind::Type),
                named!("enum_declaration", SymbolKind::Type),
                DeclRule {
                    kind: "formal_parameter",
                    bind: Bind::Field("name"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                named!("initialized_variable_definition", SymbolKind::Variable),
                named!("initialized_identifier", SymbolKind::Variable),
            ],
            ScopeLang::ObjC => &[
                DeclRule {
                    kind: "function_definition",
                    bind: Bind::Declarator("declarator"),
                    sym: SymbolKind::Function,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "declaration",
                    bind: Bind::DeclaratorChildren,
                    sym: SymbolKind::Variable,
                    ty: Ty::Own("type"),
                },
                DeclRule {
                    kind: "parameter_declaration",
                    bind: Bind::Declarator("declarator"),
                    sym: SymbolKind::Parameter,
                    ty: Ty::Own("type"),
                },
                // `@interface Star : NSObject` — the class name is the first
                // identifier; the one after it is the superclass.
                DeclRule {
                    kind: "class_interface",
                    bind: Bind::FirstIdentifierChild,
                    sym: SymbolKind::Type,
                    ty: Ty::None,
                },
                DeclRule {
                    kind: "class_implementation",
                    bind: Bind::FirstIdentifierChild,
                    sym: SymbolKind::Type,
                    ty: Ty::None,
                },
                // An Objective-C selector is split across its arguments
                // (`isBright:limit:`), so this offers the first keyword — which
                // is the part someone types to reach it.
                DeclRule {
                    kind: "method_declaration",
                    bind: Bind::FirstIdentifierChild,
                    sym: SymbolKind::Method,
                    ty: Ty::None,
                },
                named!("type_definition", SymbolKind::Type),
                named!("struct_specifier", SymbolKind::Type),
                named!("enum_specifier", SymbolKind::Type),
            ],
        }
    }

    fn is_scope(self, kind: &str) -> bool {
        self.scope_kinds().contains(&kind)
    }
}

/// Symbols visible at `byte`, nearest scope first, de-duplicated by name.
///
/// Shadowing falls out of the ordering: an inner binding is reached first, and
/// the outer one with the same name is dropped as a duplicate.
/// Collect the file's global scope. Runs on the syntax worker, beside the
/// parse that produced `tree`.
///
/// This is the whole cost of completion's scope walk — 8.7 ms on a 50k-line
/// file, and 8.7 ms whether the caret is nested five deep or sitting at byte
/// 0. Doing it here means the main thread never does it at all.
pub fn collect_global_symbols(tree: &Tree, src: &str, lang: ScopeLang) -> Vec<Found> {
    let mut out = Vec::new();
    let root = tree.root_node();
    collect_in_scope(root, src, lang, root, &mut out);
    out
}

/// The global scope's symbols, held across calls.
///
/// Measured on a 50k-line, 3,194-symbol Rust file: `visible_at` costs 8.22 ms
/// with the caret deep inside a function body and 8.73 ms with the caret at
/// byte 0, where the scope chain is one level and there are no locals at all.
/// The chain is free; the global collection is the entire cost, and it is the
/// same answer for every caret in the file. It changes only when the tree is
/// replaced, which `SyntaxEngine::live_tree_gen` reports.
#[derive(Default)]
pub struct GlobalScopeCache {
    tree_gen: u64,
    /// `None` distinguishes "not yet computed" from "computed, and empty".
    symbols: Option<Vec<Found>>,
}

impl GlobalScopeCache {
    /// Adopt the list the worker collected alongside a parse.
    ///
    /// The first version of this filled itself lazily on the main thread and
    /// keyed on the tree's identity. That could never hit: completion
    /// activates on the second character of an identifier, the characters
    /// before it reparse the file, so every activation lands just after a new
    /// tree. It measured 34x faster in a test that held the tree still and 0x
    /// faster in the app. Filling it from the worker removes the question.
    pub fn adopt(&mut self, symbols: Vec<Found>, tree_gen: u64) {
        self.tree_gen = tree_gen;
        self.symbols = Some(symbols);
    }

    /// Drop the entry, so the next call recollects on the main thread.
    pub fn invalidate(&mut self) {
        self.symbols = None;
    }
}

/// `visible_at`, reusing the global scope's symbols when the tree has not
/// moved. Prefer this on the typing path; `visible_at` recollects every time.
pub fn visible_at_cached(
    tree: &Tree,
    src: &str,
    byte: usize,
    lang: ScopeLang,
    cache: &mut GlobalScopeCache,
    tree_gen: u64,
) -> Vec<ScopeSymbol> {
    visible_at_inner(tree, src, byte, lang, Some((cache, tree_gen)))
}

pub fn visible_at(tree: &Tree, src: &str, byte: usize, lang: ScopeLang) -> Vec<ScopeSymbol> {
    visible_at_inner(tree, src, byte, lang, None)
}

fn visible_at_inner(
    tree: &Tree,
    src: &str,
    byte: usize,
    lang: ScopeLang,
    mut cache: Option<(&mut GlobalScopeCache, u64)>,
) -> Vec<ScopeSymbol> {
    let root = tree.root_node();
    let byte = byte.min(src.len());

    // The caret sits BETWEEN characters, and an empty range at the end of a
    // line belongs to no node — `descendant_for_byte_range(b, b)` answers with
    // the root, which strips every enclosing scope away.
    //
    // That position is not an edge case, it is the one that matters: it is
    // where the caret is while you type a prefix and ask for completions.
    // Rust and C hid the problem because a closing `}` keeps the caret inside
    // the block's span; Python has no such character, so completion inside a
    // function offered `def` names and nothing else — no locals, no parameters.
    //
    // So resolve against the character BEFORE the caret when the exact position
    // degenerates to the root. That is the token being typed, which is what
    // completion is asking about. The trade is that a caret just past a closing
    // brace can still see that block's bindings; over-offering there is a far
    // smaller error than losing every local at the moment of use.
    // The retry is unconditional, and that matters. It used to fire only when
    // the first attempt collapsed all the way to the root (`len() <= 1`), on
    // the assumption that anything else had found the real chain. It had not:
    // in Lua, a caret at the end of `return limit > 1` resolves to the
    // FUNCTION — two scopes, so the guard was satisfied — while the block that
    // holds every local in the body is skipped, because an empty range at a
    // node's end boundary is contained by neither the number nor the statement
    // nor the block, only by something wider. Every local in the file was
    // missing and the guard could not tell.
    //
    // So: resolve both ways and keep whichever sees more scopes. Scopes nest,
    // so "more" is "nearer the caret", never a different branch of the tree.
    let mut chain = scope_chain(root, byte, byte, lang);
    if byte > 0 {
        let prev = prev_char_boundary(src, byte);
        let retry = scope_chain(root, prev, byte, lang);
        if retry.len() > chain.len() {
            chain = retry;
        }
    }
    recover_through_error(&mut chain, root, src, byte, lang);

    let mut out: Vec<ScopeSymbol> = Vec::new();
    // A set, not a `Vec` with a linear scan: `out` already carries the order,
    // so the only thing the list was providing was membership — at O(n) a
    // lookup, over every symbol in every enclosing scope including the global
    // one. Quadratic in the count of globals, on the typing path.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let last = chain.len().saturating_sub(1);
    for (depth, scope) in chain.iter().enumerate() {
        let global = depth == last;
        // Only the global scope is worth caching, and it is the only one that
        // can be: an inner scope is identified by its node, which does not
        // survive a reparse, while the outermost one is just "the file".
        let found: Vec<Found> = match (global, cache.as_mut()) {
            (true, Some((c, wanted))) => {
                if c.tree_gen != *wanted {
                    c.tree_gen = *wanted;
                    c.symbols = None;
                }
                c.symbols
                    .get_or_insert_with(|| {
                        let mut v = Vec::new();
                        collect_in_scope(*scope, src, lang, *scope, &mut v);
                        v
                    })
                    .clone()
            }
            _ => {
                let mut v = Vec::new();
                collect_in_scope(*scope, src, lang, *scope, &mut v);
                v
            }
        };
        // A function declared directly in a type's body is a method. Only
        // grammars that spell both the same way need this; the rest already
        // said `method_definition` in their table.
        let methods_here = lang.method_owner_kinds().contains(&scope.kind());
        for (name, kind, ty) in found {
            if name.is_empty() || seen.contains(&name) {
                continue;
            }
            let kind = if methods_here && kind == SymbolKind::Function {
                SymbolKind::Method
            } else {
                kind
            };
            seen.insert(name.clone());
            out.push(ScopeSymbol {
                name,
                kind,
                ty,
                global,
                depth,
            });
        }
    }
    out
}

/// Enclosing scopes for `[from, to]`, innermost first, always ending at the
/// root.
fn scope_chain<'t>(root: Node<'t>, from: usize, to: usize, lang: ScopeLang) -> Vec<Node<'t>> {
    let start = root.descendant_for_byte_range(from, to).unwrap_or(root);
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
    chain
}

/// Re-attach the caret to a body the parser gave up on.
///
/// A half-typed identifier at the end of a block can be lifted OUT of that
/// block by error recovery. Lua does exactly this: `block` closes at the last
/// complete statement and the fragment becomes an `ERROR` sibling of the block,
/// so the scope chain jumps straight from the function to the file and every
/// local in the body vanishes — at precisely the moment completion is asked for
/// them. Rust and C never show it because their bodies are closed by a brace
/// that keeps the caret inside.
///
/// So: when the caret really is inside text the parser could not attach, and
/// the innermost scope on the chain has a scope child that closed before it,
/// that child is where the caret lexically is. Over-offering here — a binding
/// from a block that just ended — is a far smaller error than offering nothing.
fn recover_through_error<'t>(
    chain: &mut Vec<Node<'t>>,
    root: Node<'t>,
    src: &str,
    byte: usize,
    lang: ScopeLang,
) {
    let Some(&inner) = chain.first() else {
        return;
    };
    if !caret_is_in_unparsed_text(root, src, byte) {
        return;
    }
    let mut cursor = inner.walk();
    let mut candidate = None;
    for child in inner.children(&mut cursor) {
        if child.end_byte() <= byte && lang.is_scope(child.kind()) {
            candidate = Some(child);
        }
    }
    if let Some(c) = candidate {
        chain.insert(0, c);
    }
}

/// Is the caret sitting in an `ERROR` node — text the grammar could not place?
fn caret_is_in_unparsed_text(root: Node, src: &str, byte: usize) -> bool {
    // The caret is BETWEEN characters, so ask about the character before it;
    // an empty range at a boundary resolves to the root and answers nothing.
    let from = if byte > 0 {
        prev_char_boundary(src, byte)
    } else {
        byte
    };
    let Some(node) = root.descendant_for_byte_range(from, byte) else {
        return false;
    };
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n.is_error() {
            return true;
        }
        cur = n.parent();
    }
    false
}

/// Byte index of the character before `byte`.
///
/// Steps over a whole UTF-8 sequence, not one byte: slicing into the middle of
/// a multi-byte character would panic, and identifiers here are not always
/// ASCII.
fn prev_char_boundary(src: &str, byte: usize) -> usize {
    let mut i = byte.saturating_sub(1);
    while i > 0 && !src.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// One name a scope declares: identifier, what it is, and its written type.
/// One declaration as `collect_in_scope` finds it. Public because the syntax
/// worker produces these now — see `collect_global_symbols`.
pub type Found = (String, SymbolKind, Option<String>);

/// Declarations made directly inside `scope`, without entering a nested scope.
///
/// `node` walks down; `scope` is the boundary. Descending stops at any node
/// that is itself a scope (other than the boundary), which is precisely what
/// keeps one function's locals out of another's completion list.
fn collect_in_scope(node: Node, src: &str, lang: ScopeLang, scope: Node, out: &mut Vec<Found>) {
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

/// Names this node declares, if any — the whole per-language behaviour, read
/// off [`ScopeLang::rules`].
fn declarations(node: Node, src: &str, lang: ScopeLang, out: &mut Vec<Found>) {
    let kind = node.kind();
    for rule in lang.rules() {
        if rule.kind != kind {
            continue;
        }
        let ty = type_text(node, rule.ty, src);
        // A function that takes a receiver is a method, wherever it is
        // declared. Grammars that already have a separate node for it
        // (`method_definition`, `method_declaration`) said so in the table.
        let sym = if rule.sym == SymbolKind::Function && has_receiver(node, src) {
            SymbolKind::Method
        } else {
            rule.sym
        };
        match rule.bind {
            Bind::Field(f) => {
                if let Some(n) = node.child_by_field_name(f) {
                    push_text(n, src, sym, ty, out);
                }
            }
            // `__self__` means the node itself is the pattern — a grammar that
            // wraps the binding without giving it a field name.
            Bind::Pattern("__self__") => pattern_names(node, src, sym, ty, out),
            Bind::Pattern(f) => {
                if let Some(p) = node.child_by_field_name(f) {
                    pattern_names(p, src, sym, ty, out);
                }
            }
            Bind::Declarator(f) => {
                if let Some(d) = node.child_by_field_name(f) {
                    if let Some(n) = innermost_declarator_name(d, src) {
                        out.push((n, sym, ty));
                    }
                }
            }
            Bind::DeclaratorChildren => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind().ends_with("declarator") || child.kind() == "identifier" {
                        if let Some(n) = innermost_declarator_name(child, src) {
                            out.push((n, sym, ty.clone()));
                        }
                    }
                }
            }
            Bind::SelfWhenParent(parents) => {
                if node.parent().is_some_and(|p| parents.contains(&p.kind())) {
                    push_text(node, src, sym, ty, out);
                }
            }
            Bind::FirstIdentifierChild => {
                let mut cursor = node.walk();
                if let Some(n) = node
                    .named_children(&mut cursor)
                    .find(|c| c.kind() == "identifier")
                {
                    push_text(n, src, sym, ty, out);
                }
            }
            Bind::BuriedName => {
                if let Some(n) = buried_name(node, lang) {
                    push_text(n, src, sym, ty, out);
                }
            }
        }
    }
}

/// The nearest `name` field at or under `node`, without entering a scope.
///
/// For grammars that wrap the name in a signature node instead of putting it
/// on the declaration. Stopping at scope boundaries is what keeps it from
/// walking into the body and returning a local's name as the function's.
fn buried_name<'t>(node: Node<'t>, lang: ScopeLang) -> Option<Node<'t>> {
    if let Some(n) = node.child_by_field_name("name") {
        return Some(n);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if lang.is_scope(child.kind()) {
            continue;
        }
        if let Some(n) = buried_name(child, lang) {
            return Some(n);
        }
    }
    None
}

/// Does this function declare a receiver — `self`, `&self`, `this`?
///
/// This, not the enclosing type, is what makes a function a method. The
/// popup's whole complaint was that everything said `fn`; getting it the other
/// way round and calling `Star::new` a method would be the same imprecision
/// pointed the other way. Rust spells the receiver as its own node kind;
/// Python and PHP pass it as an ordinary first parameter.
fn has_receiver(node: Node, src: &str) -> bool {
    let params = ["parameters", "parameter_list", "formal_parameters"]
        .iter()
        .find_map(|f| node.child_by_field_name(f))
        .or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|c| c.kind() == "parameters" || c.kind() == "parameter_list")
        });
    let Some(params) = params else {
        return false;
    };
    let Some(first) = params.named_child(0) else {
        return false;
    };
    first.kind() == "self_parameter"
        || matches!(
            first.utf8_text(src.as_bytes()).unwrap_or(""),
            "self" | "&self" | "&mut self" | "$this"
        )
}

fn push_text(n: Node, src: &str, sym: SymbolKind, ty: Option<String>, out: &mut Vec<Found>) {
    if let Ok(t) = n.utf8_text(src.as_bytes()) {
        out.push((t.to_string(), sym, ty));
    }
}

/// The written type for a declaration, when the rule says where to look.
///
/// Capped in length: the detail column is one line beside the name, and a
/// four-line generic signature there is worse than no annotation at all.
fn type_text(node: Node, ty: Ty, src: &str) -> Option<String> {
    let field = match ty {
        Ty::None => return None,
        Ty::Own(f) => node.child_by_field_name(f),
        Ty::Parent(f) => node.parent().and_then(|p| p.child_by_field_name(f)),
    }?;
    let mut text = field.utf8_text(src.as_bytes()).ok()?.trim();
    // TypeScript's `type_annotation` node spans the colon too, so the raw text
    // is ": number". The popup wants the type, not the punctuation that
    // introduces it.
    for lead in [":", "->", "::"] {
        if let Some(rest) = text.strip_prefix(lead) {
            text = rest.trim_start();
        }
    }
    if text.is_empty() || text.len() > 40 || text.contains('\n') {
        return None;
    }
    Some(text.to_string())
}

/// Every identifier bound by a pattern — `let (a, b)` binds both.
fn pattern_names(
    node: Node,
    src: &str,
    kind: SymbolKind,
    ty: Option<String>,
    out: &mut Vec<Found>,
) {
    if matches!(
        node.kind(),
        "identifier"
            | "shorthand_property_identifier_pattern"
            | "simple_identifier"
            | "variable_name"
            // Ruby spells a capitalised name `constant`, not `identifier` —
            // so `SHARED_LIMIT = 10` at file level bound nothing at all.
            | "constant"
    ) {
        push_text(node, src, kind, ty, out);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        pattern_names(child, src, kind, ty.clone(), out);
    }
}

/// C declarators nest: `*foo[3]` -> the identifier at the centre.
fn innermost_declarator_name(node: Node, src: &str) -> Option<String> {
    if matches!(
        node.kind(),
        "identifier" | "field_identifier" | "type_identifier"
    ) {
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
