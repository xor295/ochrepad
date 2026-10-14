//! The Function List's symbols: what a document defines (functions, types,
//! modules, and the headings of a Markdown file), found from the scopes its
//! syntax gives names in definitions (`entity.name.function`,
//! `entity.name.class`, `entity.name.section`, …). No parser per language:
//! whatever a syntax marks is listed.

use std::sync::OnceLock;

use ochrepad_core::Document;
use syntect::parsing::{BasicScopeStackOp, ParseState, Scope, ScopeStack};

use crate::{MAX_HIGHLIGHT_LINE, Registry};

/// What a symbol names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    /// A class, struct, enum, trait, interface, type or impl.
    Type,
    /// A module or namespace.
    Module,
    /// A heading (Markdown and other markup).
    Heading,
}

/// One entry of the Function List.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    /// The line it is defined on.
    pub line: usize,
    pub name: String,
    pub kind: SymbolKind,
}

/// The scope prefixes that name a definition, with what they define.
fn selectors() -> &'static [(Scope, SymbolKind)] {
    static SELECTORS: OnceLock<Vec<(Scope, SymbolKind)>> = OnceLock::new();
    SELECTORS.get_or_init(|| {
        let table = [
            ("entity.name.function", SymbolKind::Function),
            ("entity.name.method", SymbolKind::Function),
            ("entity.name.class", SymbolKind::Type),
            ("entity.name.struct", SymbolKind::Type),
            ("entity.name.enum", SymbolKind::Type),
            ("entity.name.union", SymbolKind::Type),
            ("entity.name.trait", SymbolKind::Type),
            ("entity.name.interface", SymbolKind::Type),
            ("entity.name.type", SymbolKind::Type),
            ("entity.name.impl", SymbolKind::Type),
            ("entity.name.namespace", SymbolKind::Module),
            ("entity.name.module", SymbolKind::Module),
            ("entity.name.section", SymbolKind::Heading),
        ];
        table
            .iter()
            .filter_map(|(s, k)| Scope::new(s).ok().map(|scope| (scope, *k)))
            .collect()
    })
}

fn kind_of(scope: Scope) -> Option<SymbolKind> {
    selectors()
        .iter()
        .find(|(prefix, _)| prefix.is_prefix_of(scope))
        .map(|(_, k)| *k)
}

/// Whether `scope` names what an `impl` block is for (Rust), which is
/// listed as "impl Name" to tell it from the type itself.
fn is_impl(scope: Scope) -> bool {
    static IMPL: OnceLock<Option<Scope>> = OnceLock::new();
    IMPL.get_or_init(|| Scope::new("entity.name.impl").ok())
        .is_some_and(|prefix| prefix.is_prefix_of(scope))
}

impl Registry {
    /// The symbols `doc` defines in the language `syntax`, in order.
    /// `cancel` is asked every few hundred lines; `None` if it said stop.
    pub fn symbols(
        &self,
        syntax: &str,
        doc: &Document,
        cancel: &dyn Fn() -> bool,
    ) -> Option<Vec<Symbol>> {
        let mut parse = ParseState::new(self.syntax(syntax));
        let mut stack = ScopeStack::new();
        let mut out = Vec::new();
        for line in 0..doc.line_count() {
            if line % 512 == 0 && cancel() {
                return None;
            }
            if doc.line_end(line).0 - doc.line_start(line).0 > MAX_HIGHLIGHT_LINE {
                continue;
            }
            let mut text = doc.line_text(line);
            text.push('\n');
            let Ok(ops) = parse.parse_line(&text, &self.syntaxes) else {
                break;
            };
            // The definition being named: the stack depth it was pushed
            // at, where its text starts, its kind, and whether it is an
            // impl.
            let mut open: Option<(usize, usize, SymbolKind, bool)> = None;
            for (at, op) in ops {
                let mut closed = None;
                let _ = stack.apply_with_hook(&op, |basic, scopes| match basic {
                    BasicScopeStackOp::Push(scope) => {
                        if open.is_none()
                            && let Some(kind) = kind_of(scope)
                        {
                            open = Some((scopes.len(), at, kind, is_impl(scope)));
                        }
                    }
                    BasicScopeStackOp::Pop => {
                        if let Some((depth, start, kind, imp)) = open
                            && scopes.len() < depth
                        {
                            closed = Some((start, kind, imp));
                            open = None;
                        }
                    }
                });
                if let Some((start, kind, imp)) = closed {
                    push_symbol(&mut out, line, &text, start..at, kind, imp);
                }
            }
            // Named up to the end of the line.
            if let Some((_, start, kind, imp)) = open {
                push_symbol(&mut out, line, &text, start..text.len(), kind, imp);
            }
        }
        Some(out)
    }
}

fn push_symbol(
    out: &mut Vec<Symbol>,
    line: usize,
    text: &str,
    range: std::ops::Range<usize>,
    kind: SymbolKind,
    imp: bool,
) {
    let Some(name) = text.get(range) else {
        return;
    };
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let name = if imp {
        format!("impl {name}")
    } else {
        name.to_owned()
    };
    let name = name.as_str();
    // A name split by nested scopes on one line comes as one symbol.
    if let Some(last) = out.last()
        && last.line == line
        && last.kind == kind
        && last.name == name
    {
        return;
    }
    out.push(Symbol {
        line,
        name: name.to_owned(),
        kind,
    });
}
