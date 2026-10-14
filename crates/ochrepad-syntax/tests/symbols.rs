//! The Function List's symbols, from the syntaxes' scopes.

use ochrepad_core::Document;
use ochrepad_syntax::{Registry, SymbolKind};

fn symbols(reg: &Registry, syntax: &str, text: &str) -> Vec<(usize, String, SymbolKind)> {
    reg.symbols(syntax, &Document::new(text), &|| false)
        .expect("not cancelled")
        .into_iter()
        .map(|s| (s.line, s.name, s.kind))
        .collect()
}

#[test]
fn rust_python_and_markdown_definitions() {
    let reg = Registry::load();
    let rust = "mod net {\n    pub struct Server;\n    impl Server {\n        pub fn start(&self) {\n            helper();\n        }\n    }\n}\nfn main() {}\nenum Mode { A }\ntrait Run {}\n";
    let got = symbols(&reg, "Rust", rust);
    let names: Vec<(usize, &str, SymbolKind)> =
        got.iter().map(|(l, n, k)| (*l, n.as_str(), *k)).collect();
    assert!(names.contains(&(0, "net", SymbolKind::Module)), "{names:?}");
    assert!(
        names.contains(&(1, "Server", SymbolKind::Type)),
        "{names:?}"
    );
    assert!(
        names.contains(&(2, "impl Server", SymbolKind::Type)),
        "{names:?}"
    );
    assert!(
        names.contains(&(3, "start", SymbolKind::Function)),
        "{names:?}"
    );
    assert!(
        names.contains(&(8, "main", SymbolKind::Function)),
        "{names:?}"
    );
    assert!(names.contains(&(9, "Mode", SymbolKind::Type)), "{names:?}");
    assert!(names.contains(&(10, "Run", SymbolKind::Type)), "{names:?}");
    // A call is not a definition.
    assert!(!names.iter().any(|(_, n, _)| *n == "helper"), "{names:?}");

    let python = "class Shape:\n    def area(self):\n        return 0\n\ndef main():\n    pass\n";
    let got = symbols(&reg, "Python", python);
    assert_eq!(
        got,
        vec![
            (0, "Shape".to_owned(), SymbolKind::Type),
            (1, "area".to_owned(), SymbolKind::Function),
            (4, "main".to_owned(), SymbolKind::Function),
        ]
    );

    let markdown = "# Title\n\nSome text.\n\n## Install\n\nMore.\n";
    let got = symbols(&reg, "Markdown", markdown);
    let headings: Vec<&str> = got.iter().map(|(_, n, _)| n.as_str()).collect();
    assert_eq!(headings, vec!["Title", "Install"], "{got:?}");
    assert!(got.iter().all(|s| s.2 == SymbolKind::Heading));

    assert!(symbols(&reg, "Plain Text", "fn main() {}\n").is_empty());
}

#[test]
fn a_cancelled_search_returns_nothing() {
    let reg = Registry::load();
    let doc = Document::new(&"fn f() {}\n".repeat(2000));
    assert!(reg.symbols("Rust", &doc, &|| true).is_none());
}
