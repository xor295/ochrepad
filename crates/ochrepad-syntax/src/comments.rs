//! Comment markers per language, for Toggle Comment. syntect can read them
//! from a syntax's metadata, but the two-face syntax set does not carry it,
//! so this is a table keyed by the language names `Registry` uses.

use ochrepad_core::CommentSyntax;

type Entry = (
    &'static str,
    Option<&'static str>,
    Option<(&'static str, &'static str)>,
);

const C: Option<(&str, &str)> = Some(("/*", "*/"));
const HTML: Option<(&str, &str)> = Some(("<!--", "-->"));
const ML: Option<(&str, &str)> = Some(("(*", "*)"));
const HASKELL: Option<(&str, &str)> = Some(("{-", "-}"));

const TABLE: &[Entry] = &[
    // C-like: `//` and `/* */`.
    ("ActionScript", Some("//"), C),
    ("C", Some("//"), C),
    ("C#", Some("//"), C),
    ("C++", Some("//"), C),
    ("D", Some("//"), C),
    ("Dart", Some("//"), C),
    ("GLSL", Some("//"), C),
    ("Go", Some("//"), C),
    ("Graphviz (DOT)", Some("//"), C),
    ("Groovy", Some("//"), C),
    ("Java", Some("//"), C),
    ("JavaScript", Some("//"), C),
    ("jsonnet", Some("//"), C),
    ("Kotlin", Some("//"), C),
    ("Less", Some("//"), C),
    ("Objective-C", Some("//"), C),
    ("Objective-C++", Some("//"), C),
    ("Odin", Some("//"), C),
    ("PHP", Some("//"), C),
    ("Protocol Buffer", Some("//"), C),
    ("QML", Some("//"), C),
    ("Rust", Some("//"), C),
    ("Sass", Some("//"), C),
    ("Scala", Some("//"), C),
    ("SCSS", Some("//"), C),
    ("Solidity", Some("//"), C),
    ("Stylus", Some("//"), C),
    ("Swift", Some("//"), C),
    ("SystemVerilog", Some("//"), C),
    ("TypeScript", Some("//"), C),
    ("TypeScriptReact", Some("//"), C),
    ("Typst", Some("//"), C),
    ("Verilog", Some("//"), C),
    ("WGSL", Some("//"), C),
    ("Zig", Some("//"), None),
    ("F#", Some("//"), ML),
    ("Pascal", Some("//"), Some(("{", "}"))),
    ("CSS", None, C),
    // `#` line comments.
    ("AWK", Some("#"), None),
    ("Apache Conf", Some("#"), None),
    ("Bourne Again Shell (bash)", Some("#"), None),
    ("CMake", Some("#"), None),
    ("CoffeeScript", Some("#"), None),
    ("Crontab", Some("#"), None),
    ("Crystal", Some("#"), None),
    ("Dockerfile", Some("#"), None),
    ("Dockerfile (with bash)", Some("#"), None),
    ("DotENV", Some("#"), None),
    ("Elixir", Some("#"), None),
    ("Fish", Some("#"), None),
    ("GDScript (Godot Engine)", Some("#"), None),
    ("Git Attributes", Some("#"), None),
    ("Git Config", Some("#"), None),
    ("Git Ignore", Some("#"), None),
    ("gnuplot", Some("#"), None),
    ("GraphQL", Some("#"), None),
    ("Java Properties", Some("#"), None),
    ("Julia", Some("#"), Some(("#=", "=#"))),
    ("LiveScript", Some("#"), None),
    ("Makefile", Some("#"), None),
    ("nginx", Some("#"), None),
    ("Nim", Some("#"), Some(("#[", "]#"))),
    ("Ninja", Some("#"), None),
    ("Nix", Some("#"), C),
    ("Perl", Some("#"), None),
    ("Puppet", Some("#"), None),
    ("Python", Some("#"), None),
    ("R", Some("#"), None),
    ("Rego", Some("#"), None),
    ("Requirements.txt", Some("#"), None),
    ("Robot Framework", Some("#"), None),
    ("Ruby", Some("#"), None),
    ("SSH Config", Some("#"), None),
    ("SSHD Config", Some("#"), None),
    ("Tcl", Some("#"), None),
    ("Terraform", Some("#"), C),
    ("TOML", Some("#"), None),
    ("Vyper", Some("#"), None),
    ("YAML", Some("#"), None),
    // `--`.
    ("Ada", Some("--"), None),
    ("AppleScript", Some("--"), ML),
    ("Elm", Some("--"), HASKELL),
    ("Haskell", Some("--"), HASKELL),
    ("Idris", Some("--"), HASKELL),
    ("Lean 4", Some("--"), Some(("/-", "-/"))),
    ("Lua", Some("--"), Some(("--[[", "]]"))),
    ("PureScript", Some("--"), HASKELL),
    ("SQL", Some("--"), C),
    ("VHDL", Some("--"), None),
    // `;`.
    ("Clojure", Some(";"), None),
    ("INI", Some(";"), None),
    ("Lisp", Some(";"), None),
    ("LLVM", Some(";"), None),
    ("NSIS", Some(";"), C),
    ("Racket", Some(";"), Some(("#|", "|#"))),
    ("x86_64 Assembly", Some(";"), None),
    // `%`.
    ("Erlang", Some("%"), None),
    ("LaTeX", Some("%"), None),
    ("MATLAB", Some("%"), Some(("%{", "%}"))),
    ("TeX", Some("%"), None),
    // Others.
    ("Batch File", Some("REM"), None),
    ("Fortran (Modern)", Some("!"), None),
    ("VimL", Some("\""), None),
    ("OCaml", None, ML),
    ("SML", None, ML),
    ("HTML", None, HTML),
    ("Markdown", None, HTML),
    ("MultiMarkdown", None, HTML),
    ("Svelte", None, HTML),
    ("Vue Component", None, HTML),
    ("XML", None, HTML),
];

/// The comment markers of `language` (a `Registry` language name), if
/// known.
pub fn comment_syntax(language: &str) -> Option<CommentSyntax> {
    let &(_, line, block) = TABLE.iter().find(|(name, _, _)| *name == language)?;
    Some(CommentSyntax {
        line: line.map(str::to_owned),
        block: block.map(|(a, b)| (a.to_owned(), b.to_owned())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_names_a_known_language() {
        let names = crate::Registry::load().language_names();
        for (name, line, block) in TABLE {
            assert!(names.iter().any(|n| n == name), "unknown language {name}");
            assert!(line.is_some() || block.is_some(), "{name} has no markers");
        }
        assert_eq!(
            comment_syntax("Python").and_then(|c| c.line),
            Some("#".into())
        );
        assert!(comment_syntax("Plain Text").is_none());
    }
}
