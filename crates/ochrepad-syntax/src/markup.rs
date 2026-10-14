//! The markup languages, whose matching tags the editor outlines, and how
//! each pairs its tags. Like the comment markers, a table keyed by the
//! language names `Registry` uses.

/// How a markup language's tags pair up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Markup {
    /// HTML: void elements such as `<br>` have no end tag, names match in
    /// any case, and `<script>` and `<style>` hold text, not tags.
    Html,
    /// XML: every element ends with an end tag of exactly its name, or `/>`.
    Xml,
}

const TABLE: &[(&str, Markup)] = &[
    // Also XHTML (`.xhtml`).
    ("HTML", Markup::Html),
    // HTML with a template language's code in it, as Notepad++ matches
    // tags in PHP, ASP and JSP.
    ("HTML (ASP)", Markup::Html),
    ("HTML (EEx)", Markup::Html),
    ("HTML (Erlang)", Markup::Html),
    ("HTML (Jinja2)", Markup::Html),
    ("HTML (Rails)", Markup::Html),
    ("HTML (Tcl)", Markup::Html),
    ("HTML (Twig)", Markup::Html),
    ("Java Server Page (JSP)", Markup::Html),
    ("PHP", Markup::Html),
    ("Svelte", Markup::Html),
    ("Vue Component", Markup::Html),
    // Also SVG, XSLT, XSD, RSS, XAML and other XML formats.
    ("XML", Markup::Xml),
];

/// How `language` (a `Registry` language name) pairs its tags, if it is
/// markup.
pub fn markup(language: &str) -> Option<Markup> {
    TABLE
        .iter()
        .find(|(name, _)| *name == language)
        .map(|&(_, m)| m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_names_a_known_language() {
        let names = crate::Registry::load().language_names();
        for (name, _) in TABLE {
            assert!(names.iter().any(|n| n == name), "unknown language {name}");
        }
        assert_eq!(markup("Markdown"), None);
        assert_eq!(markup("Plain Text"), None);
    }

    #[test]
    fn markup_files_are_found_by_name_and_first_line() {
        let reg = crate::Registry::load();
        let of = |file: &str, first_line: &str| markup(&reg.detect(Some(file), first_line));
        for file in ["a.html", "a.htm", "a.xhtml", "a.php", "a.vue", "a.jsp"] {
            assert_eq!(of(file, ""), Some(Markup::Html), "{file}");
        }
        for file in ["a.xml", "a.svg", "a.xslt", "a.xsd", "a.rss", "a.xaml"] {
            assert_eq!(of(file, ""), Some(Markup::Xml), "{file}");
        }
        assert_eq!(of("a.config", "<?xml version=\"1.0\"?>"), Some(Markup::Xml));
        assert_eq!(of("a.md", ""), None);
    }
}
