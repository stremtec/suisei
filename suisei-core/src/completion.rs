pub struct Completions {
    pub suggestions: Vec<Suggestion>,
    pub selected: usize,
    pub active: bool,
    pub prefix: String,
}

#[derive(Clone, Debug)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    pub insert_text: String,
}

impl Default for Completions {
    fn default() -> Self {
        Self {
            suggestions: Vec::new(),
            selected: 0,
            active: false,
            prefix: String::new(),
        }
    }
}

impl Completions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn activate(&mut self, prefix: &str, ext: Option<&str>) {
        self.prefix = prefix.to_string();
        self.selected = 0;
        self.suggestions = get_suggestions(prefix, ext);
        self.active = !self.suggestions.is_empty();
    }

    pub fn deactivate(&mut self) {
        self.active = false;
        self.suggestions.clear();
        self.selected = 0;
        self.prefix.clear();
    }

    pub fn selected_suggestion(&self) -> Option<&Suggestion> {
        self.suggestions.get(self.selected)
    }

    pub fn next(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected = (self.selected + 1) % self.suggestions.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.suggestions.is_empty() {
            if self.selected == 0 {
                self.selected = self.suggestions.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    pub fn refine(&mut self, prefix: &str) {
        self.prefix = prefix.to_string();
        let prefix_lower = prefix.to_lowercase();
        let prev_count = self.suggestions.len();
        self.suggestions.retain(|s| s.label.to_lowercase().starts_with(&prefix_lower));
        if self.suggestions.len() < prev_count {
            self.selected = 0;
        }
        if self.suggestions.is_empty() {
            self.active = false;
        }
    }
}

fn get_suggestions(prefix: &str, ext: Option<&str>) -> Vec<Suggestion> {
    let keywords = match ext {
        Some("rs") => rust_keywords(),
        Some("ts" | "tsx") => ts_keywords(),
        Some("js" | "jsx") => js_keywords(),
        Some("py") => py_keywords(),
        Some("go") => go_keywords(),
        Some("html" | "htm") => html_keywords(),
        Some("css") => css_keywords(),
        Some("json") => json_keywords(),
        Some("toml") => toml_keywords(),
        Some("md" | "mdx") => markdown_keywords(),
        Some("sh" | "bash" | "zsh") => shell_keywords(),
        Some("yaml" | "yml") => yaml_keywords(),
        Some("sql") => sql_keywords(),
        Some("c" | "h") => c_keywords(),
        Some("cpp" | "hpp" | "cc" | "cxx") => cpp_keywords(),
        _ => vec![],
    };

    let prefix_lower = prefix.to_lowercase();
    keywords
        .into_iter()
        .filter(|(label, _)| label.to_lowercase().starts_with(&prefix_lower))
        .map(|(label, detail)| Suggestion {
            insert_text: label.to_string(),
            label: label.to_string(),
            detail: detail.to_string(),
        })
        .collect()
}

fn rust_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("fn", "function"),
        ("let", "variable binding"),
        ("let mut", "mutable binding"),
        ("struct", "struct"),
        ("impl", "impl block"),
        ("enum", "enum"),
        ("trait", "trait"),
        ("impl", "implementation"),
        ("pub", "public visibility"),
        ("pub fn", "public function"),
        ("pub(crate)", "crate visibility"),
        ("use", "import"),
        ("mod", "module"),
        ("match", "match expression"),
        ("if", "if expression"),
        ("if let", "if let pattern"),
        ("else", "else branch"),
        ("while", "while loop"),
        ("for", "for loop"),
        ("loop", "infinite loop"),
        ("return", "return statement"),
        ("self", "self reference"),
        ("Self", "Self type"),
        ("const", "constant"),
        ("static", "static variable"),
        ("type", "type alias"),
        ("where", "where clause"),
        ("unsafe", "unsafe block"),
        ("async", "async function"),
        ("await", "await expression"),
        ("move", "move closure"),
        ("ref", "ref pattern"),
        ("mut", "mutable binding"),
        ("true", "boolean"),
        ("false", "boolean"),
        ("Some", "Option::Some"),
        ("None", "Option::None"),
        ("Ok", "Result::Ok"),
        ("Err", "Result::Err"),
        ("Result", "Result type"),
        ("Option", "Option type"),
        ("Vec", "Vec type"),
        ("String", "String type"),
        ("HashMap", "HashMap type"),
        ("println!", "print macro"),
        ("format!", "format macro"),
        ("vec!", "vec macro"),
        ("eprintln!", "stderr print"),
        ("dbg!", "debug macro"),
        ("assert!", "assert macro"),
        ("todo!", "todo macro"),
        ("unimplemented!", "unimplemented macro"),
        ("#[derive(", "derive attribute"),
        ("#[cfg(", "cfg attribute"),
        ("#![allow(", "allow attribute"),
        ("crate", "root module"),
        ("super", "parent module"),
        ("dyn", "dynamic dispatch"),
        ("as", "type cast"),
        ("in", "in keyword"),
        ("break", "break statement"),
        ("continue", "continue statement"),
        ("extern", "external block"),
        ("macro_rules!", "macro definition"),
        ("Box", "Box type"),
        ("Rc", "Rc type"),
        ("Arc", "Arc type"),
        ("Cell", "Cell type"),
        ("RefCell", "RefCell type"),
        ("Mutex", "Mutex type"),
        ("RwLock", "RwLock type"),
        ("Clone", "Clone trait"),
        ("Copy", "Copy trait"),
        ("Debug", "Debug trait"),
        ("Default", "Default trait"),
        ("Drop", "Drop trait"),
        ("From", "From trait"),
        ("Into", "Into trait"),
        ("Iterator", "Iterator trait"),
        ("std::", "standard library"),
    ]
}

fn ts_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("function", "function"),
        ("const", "constant"),
        ("let", "variable"),
        ("var", "variable (legacy)"),
        ("class", "class"),
        ("interface", "interface"),
        ("type", "type alias"),
        ("enum", "enum"),
        ("import", "import"),
        ("export", "export"),
        ("export default", "default export"),
        ("async", "async"),
        ("await", "await"),
        ("return", "return"),
        ("if", "if"),
        ("else", "else"),
        ("for", "for"),
        ("while", "while"),
        ("switch", "switch"),
        ("case", "case"),
        ("try", "try"),
        ("catch", "catch"),
        ("throw", "throw"),
        ("new", "new"),
        ("extends", "extends"),
        ("implements", "implements"),
        ("private", "private"),
        ("protected", "protected"),
        ("public", "public"),
        ("readonly", "readonly"),
        ("static", "static"),
        ("abstract", "abstract"),
        ("typeof", "typeof"),
        ("keyof", "keyof"),
        ("as", "type assertion"),
        ("in", "in"),
        ("null", "null"),
        ("undefined", "undefined"),
        ("true", "true"),
        ("false", "false"),
    ]
}

fn js_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("function", "function"),
        ("const", "constant"),
        ("let", "variable"),
        ("var", "variable"),
        ("class", "class"),
        ("import", "import"),
        ("export", "export"),
        ("export default", "default export"),
        ("async", "async"),
        ("await", "await"),
        ("return", "return"),
        ("if", "if"),
        ("else", "else"),
        ("for", "for"),
        ("while", "while"),
        ("switch", "switch"),
        ("try", "try"),
        ("catch", "catch"),
        ("throw", "throw"),
        ("new", "new"),
        ("null", "null"),
        ("undefined", "undefined"),
        ("true", "true"),
        ("false", "false"),
        ("console.log(", "log"),
        ("console.error(", "error"),
        ("JSON.parse(", "parse JSON"),
        ("JSON.stringify(", "stringify JSON"),
        ("Promise", "Promise"),
        ("async ", "async fn"),
        ("setTimeout(", "setTimeout"),
        ("setInterval(", "setInterval"),
        ("require(", "require"),
        ("module.exports", "module.exports"),
    ]
}

fn py_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("def", "function"),
        ("class", "class"),
        ("import", "import"),
        ("from", "import from"),
        ("if", "if"),
        ("elif", "elif"),
        ("else", "else"),
        ("for", "for"),
        ("while", "while"),
        ("try", "try"),
        ("except", "except"),
        ("finally", "finally"),
        ("raise", "raise"),
        ("with", "with"),
        ("as", "as"),
        ("return", "return"),
        ("yield", "yield"),
        ("lambda", "lambda"),
        ("async", "async"),
        ("await", "await"),
        ("pass", "pass"),
        ("break", "break"),
        ("continue", "continue"),
        ("self", "self"),
        ("True", "True"),
        ("False", "False"),
        ("None", "None"),
        ("print(", "print"),
        ("len(", "len"),
        ("range(", "range"),
        ("enumerate(", "enumerate"),
        ("zip(", "zip"),
        ("list(", "list"),
        ("dict(", "dict"),
        ("set(", "set"),
        ("tuple(", "tuple"),
        ("str(", "str"),
        ("int(", "int"),
        ("float(", "float"),
        ("type(", "type"),
        ("isinstance(", "isinstance"),
        ("super()", "super"),
        ("__init__", "__init__"),
        ("__str__", "__str__"),
        ("__repr__", "__repr__"),
    ]
}

fn go_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("func", "function"),
        ("var", "variable"),
        ("const", "constant"),
        ("type", "type"),
        ("struct", "struct"),
        ("interface", "interface"),
        ("package", "package"),
        ("import", "import"),
        ("if", "if"),
        ("else", "else"),
        ("for", "for"),
        ("range", "range"),
        ("switch", "switch"),
        ("case", "case"),
        ("default", "default"),
        ("defer", "defer"),
        ("go", "goroutine"),
        ("chan", "channel"),
        ("select", "select"),
        ("return", "return"),
        ("break", "break"),
        ("continue", "continue"),
        ("map", "map"),
        ("make(", "make"),
        ("new(", "new"),
        ("nil", "nil"),
        ("true", "true"),
        ("false", "false"),
    ]
}

fn html_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("<!DOCTYPE html>", "doctype"),
        ("<html>", "html"),
        ("<head>", "head"),
        ("<body>", "body"),
        ("<div>", "div"),
        ("<span>", "span"),
        ("<p>", "paragraph"),
        ("<a href=\"\">", "anchor"),
        ("<img src=\"\" alt=\"\">", "image"),
        ("<ul>", "unordered list"),
        ("<ol>", "ordered list"),
        ("<li>", "list item"),
        ("<table>", "table"),
        ("<tr>", "table row"),
        ("<td>", "table data"),
        ("<th>", "table header"),
        ("<form>", "form"),
        ("<input>", "input"),
        ("<button>", "button"),
        ("<script>", "script"),
        ("<style>", "style"),
        ("<link>", "link"),
        ("<meta>", "meta"),
        ("<h1>", "heading 1"),
        ("<h2>", "heading 2"),
        ("<h3>", "heading 3"),
        ("<header>", "header"),
        ("<footer>", "footer"),
        ("<nav>", "nav"),
        ("<main>", "main"),
        ("<section>", "section"),
        ("<article>", "article"),
        ("<aside>", "aside"),
    ]
}

fn css_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("color:", "text color"),
        ("background:", "background"),
        ("background-color:", "bg color"),
        ("margin:", "margin"),
        ("padding:", "padding"),
        ("border:", "border"),
        ("border-radius:", "border radius"),
        ("font-size:", "font size"),
        ("font-weight:", "font weight"),
        ("font-family:", "font family"),
        ("display:", "display"),
        ("flex", "flex container"),
        ("grid", "grid container"),
        ("position:", "position"),
        ("width:", "width"),
        ("height:", "height"),
        ("max-width:", "max width"),
        ("min-height:", "min height"),
        ("overflow:", "overflow"),
        ("opacity:", "opacity"),
        ("z-index:", "z-index"),
        ("text-align:", "text align"),
        ("line-height:", "line height"),
        ("cursor:", "cursor"),
        ("transition:", "transition"),
        ("transform:", "transform"),
        ("box-shadow:", "box shadow"),
        (":hover", "hover pseudo"),
        ("::before", "before pseudo"),
        ("::after", "after pseudo"),
        ("@media", "media query"),
        ("@import", "import"),
    ]
}

fn json_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("true", "boolean"),
        ("false", "boolean"),
        ("null", "null"),
    ]
}

fn toml_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("[package]", "package section"),
        ("[dependencies]", "dependencies"),
        ("[dev-dependencies]", "dev deps"),
        ("[build-dependencies]", "build deps"),
        ("[features]", "features"),
        ("[profile]", "profile"),
        ("[workspace]", "workspace"),
        ("name = ", "package name"),
        ("version = ", "version"),
        ("edition = ", "edition"),
    ]
}

fn markdown_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("# ", "heading 1"),
        ("## ", "heading 2"),
        ("### ", "heading 3"),
        ("#### ", "heading 4"),
        ("**", "bold"),
        ("__", "italic"),
        ("`", "code"),
        ("```", "code block"),
        ("> ", "blockquote"),
        ("- ", "list item"),
        ("1. ", "ordered list"),
        ("[text](", "link"),
        ("![alt](", "image"),
        ("---", "horizontal rule"),
        ("- [ ] ", "task"),
    ]
}

fn shell_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("#!/bin/bash", "shebang bash"),
        ("#!/bin/sh", "shebang sh"),
        ("#!/usr/bin/env bash", "shebang env bash"),
        ("if", "if"),
        ("then", "then"),
        ("else", "else"),
        ("elif", "elif"),
        ("fi", "fi (end if)"),
        ("for", "for"),
        ("while", "while"),
        ("do", "do"),
        ("done", "done"),
        ("case", "case"),
        ("esac", "esac"),
        ("function", "function"),
        ("local", "local"),
        ("export", "export"),
        ("source", "source"),
        ("exit", "exit"),
        ("return", "return"),
        ("echo", "echo"),
        ("read", "read"),
        ("test", "test"),
        ("shift", "shift"),
        ("unset", "unset"),
        ("alias", "alias"),
        ("trap", "trap"),
    ]
}

fn yaml_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("---", "document start"),
        ("...", "document end"),
        ("true", "boolean"),
        ("false", "boolean"),
        ("null", "null"),
    ]
}

fn sql_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("SELECT", "select"),
        ("FROM", "from"),
        ("WHERE", "where"),
        ("INSERT INTO", "insert"),
        ("VALUES", "values"),
        ("UPDATE", "update"),
        ("SET", "set"),
        ("DELETE", "delete"),
        ("CREATE TABLE", "create table"),
        ("ALTER TABLE", "alter table"),
        ("DROP TABLE", "drop table"),
        ("JOIN", "join"),
        ("LEFT JOIN", "left join"),
        ("INNER JOIN", "inner join"),
        ("ON", "on"),
        ("GROUP BY", "group by"),
        ("ORDER BY", "order by"),
        ("HAVING", "having"),
        ("LIMIT", "limit"),
        ("OFFSET", "offset"),
        ("INDEX", "index"),
        ("PRIMARY KEY", "primary key"),
        ("FOREIGN KEY", "foreign key"),
        ("NOT NULL", "not null"),
        ("DEFAULT", "default"),
        ("UNIQUE", "unique"),
        ("AS", "alias"),
        ("DISTINCT", "distinct"),
        ("COUNT(", "count"),
        ("SUM(", "sum"),
        ("AVG(", "avg"),
        ("MAX(", "max"),
        ("MIN(", "min"),
    ]
}

fn c_keywords() -> Vec<(&'static str, &'static str)> {
    vec![
        ("int", "integer"),
        ("char", "character"),
        ("float", "float"),
        ("double", "double"),
        ("void", "void"),
        ("struct", "struct"),
        ("union", "union"),
        ("enum", "enum"),
        ("typedef", "typedef"),
        ("sizeof", "sizeof"),
        ("if", "if"),
        ("else", "else"),
        ("for", "for"),
        ("while", "while"),
        ("do", "do"),
        ("switch", "switch"),
        ("case", "case"),
        ("break", "break"),
        ("continue", "continue"),
        ("return", "return"),
        ("static", "static"),
        ("extern", "extern"),
        ("const", "const"),
        ("volatile", "volatile"),
        ("register", "register"),
        ("auto", "auto"),
        ("unsigned", "unsigned"),
        ("signed", "signed"),
        ("short", "short"),
        ("long", "long"),
        ("#include", "include"),
        ("#define", "define"),
        ("#ifdef", "ifdef"),
        ("#ifndef", "ifndef"),
        ("#endif", "endif"),
        ("NULL", "null"),
        ("malloc(", "malloc"),
        ("free(", "free"),
        ("printf(", "printf"),
        ("scanf(", "scanf"),
    ]
}

fn cpp_keywords() -> Vec<(&'static str, &'static str)> {
    let mut keys = c_keywords();
    keys.extend(vec![
        ("class", "class"),
        ("namespace", "namespace"),
        ("public:", "public"),
        ("private:", "private"),
        ("protected:", "protected"),
        ("virtual", "virtual"),
        ("override", "override"),
        ("template", "template"),
        ("typename", "typename"),
        ("new", "new"),
        ("delete", "delete"),
        ("this", "this"),
        ("nullptr", "nullptr"),
        ("constexpr", "constexpr"),
        ("noexcept", "noexcept"),
        ("friend", "friend"),
        ("operator", "operator"),
        ("explicit", "explicit"),
        ("mutable", "mutable"),
        ("using", "using"),
        ("auto", "auto"),
        ("decltype", "decltype"),
        ("try", "try"),
        ("catch", "catch"),
        ("throw", "throw"),
        ("#include", "include"),
        ("std::", "std namespace"),
        ("std::string", "string"),
        ("std::vector", "vector"),
        ("std::map", "map"),
        ("std::cout", "cout"),
        ("std::cin", "cin"),
        ("std::unique_ptr", "unique ptr"),
        ("std::shared_ptr", "shared ptr"),
        ("std::make_unique", "make unique"),
        ("std::make_shared", "make shared"),
    ]);
    keys
}
