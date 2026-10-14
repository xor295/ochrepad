//! File > Export > HTML…, as Notepad++'s NppExport: the document as a
//! standalone web page that looks as it does in the editor, its text in the
//! current syntax theme's colours. The page is made on a thread, which
//! highlights the whole document (one over the highlighting size limit is
//! exported without colours), and the worker writes it, as it writes every
//! user file.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use ochrepad_core::Document;
use ochrepad_syntax::{LineSpans, Registry, Span, ThemeColors};

/// The page's font: the usual monospace fonts of Windows, Linux and macOS,
/// then the browser's own.
const FONTS: &str = "Consolas, \"DejaVu Sans Mono\", \"Liberation Mono\", Menlo, monospace";

/// `text` (lines separated by `\n`) as a standalone web page titled
/// `title`. Line `i` is coloured by `spans[i]` (byte ranges in the line);
/// the rest of the text, and the lines past the end of `spans`, are in the
/// theme's default colour. Tabs are `tab_width` columns wide.
pub fn html(
    title: &str,
    text: &str,
    spans: &[LineSpans],
    colors: ThemeColors,
    tab_width: usize,
) -> String {
    let background = colors.background;
    let [r, g, b, _] = background;
    let foreground = over(colors.foreground, background);
    // The text, and room for the tags.
    let mut out = String::with_capacity(text.len() + text.len() / 2 + 512);
    out.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>");
    escape_into(&mut out, title);
    out.push_str("</title>\n<style>\n");
    out.push_str(&format!(
        "body {{ margin: 0; background: {}; }}\n\
         pre {{ margin: 0; padding: 8px; color: {}; font-family: {FONTS}; tab-size: {tab_width}; }}\n",
        hex([r, g, b]),
        hex(foreground)
    ));
    out.push_str("</style>\n</head>\n<body>\n");
    // Browsers drop a line break just after <pre>: this one, so that a
    // blank first line of the text stays.
    out.push_str("<pre>\n");
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let line_spans = spans.get(i).map_or(&[][..], |s| &s[..]);
        line_into(&mut out, line, line_spans, background, foreground);
    }
    out.push_str("</pre>\n</body>\n</html>\n");
    out
}

/// Appends `line` with each of its `spans` in a `<span>`, except those that
/// look like the default text (in `foreground`, and not bold, italic or
/// underlined).
fn line_into(
    out: &mut String,
    line: &str,
    spans: &[Span],
    background: [u8; 4],
    foreground: [u8; 3],
) {
    let mut at = 0;
    for span in spans {
        let start = span.start.max(at);
        // A span that does not fall on the line's characters is left out.
        let Some(run) = line.get(start..span.end.min(line.len())) else {
            continue;
        };
        if run.is_empty() {
            continue;
        }
        escape_into(out, &line[at..start]);
        let color = over(span.color, background);
        if color == foreground && !span.bold && !span.italic && !span.underline {
            escape_into(out, run);
        } else {
            out.push_str("<span style=\"color:");
            out.push_str(&hex(color));
            if span.bold {
                out.push_str(";font-weight:bold");
            }
            if span.italic {
                out.push_str(";font-style:italic");
            }
            if span.underline {
                out.push_str(";text-decoration:underline");
            }
            out.push_str("\">");
            escape_into(out, run);
            out.push_str("</span>");
        }
        at = start + run.len();
    }
    escape_into(out, &line[at..]);
}

/// Appends `text` with `&`, `<`, `>` and `"` escaped.
fn escape_into(out: &mut String, text: &str) {
    let mut from = 0;
    for (i, byte) in text.bytes().enumerate() {
        let entity = match byte {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            _ => continue,
        };
        out.push_str(&text[from..i]);
        out.push_str(entity);
        from = i + 1;
    }
    out.push_str(&text[from..]);
}

/// What `color` (straight RGBA) looks like over `background`: itself where
/// it is opaque, mixed with the background where it is not.
fn over(color: [u8; 4], background: [u8; 4]) -> [u8; 3] {
    let alpha = u32::from(color[3]);
    let mix = |c: u8, under: u8| {
        ((u32::from(c) * alpha + u32::from(under) * (255 - alpha) + 127) / 255) as u8
    };
    [
        mix(color[0], background[0]),
        mix(color[1], background[1]),
        mix(color[2], background[2]),
    ]
}

/// `#rrggbb`.
fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// What to export: the document as it was when the export was chosen.
pub struct Request {
    pub doc: Document,
    /// The page's title: the document's name.
    pub title: String,
    /// The language to highlight it in; `None` exports it without colours.
    pub language: Option<String>,
    /// The syntax theme whose colours the page takes.
    pub theme: String,
    pub tab_width: usize,
}

/// A page being made on a thread.
pub struct Export {
    /// Where the page goes.
    pub path: PathBuf,
    /// Whether it gets the highlighting's colours.
    pub colored: bool,
    rx: Receiver<Document>,
}

impl Export {
    /// Starts making the page for `request`; `None` if no thread could be
    /// started.
    pub fn start(registry: &Arc<Registry>, request: Request, path: PathBuf) -> Option<Self> {
        let (tx, rx) = channel();
        let registry = registry.clone();
        let colored = request.language.is_some();
        std::thread::Builder::new()
            .name("ochrepad-export".into())
            .spawn(move || {
                let Request {
                    doc,
                    title,
                    language,
                    theme,
                    tab_width,
                } = request;
                let spans = language.map_or_else(Vec::new, |language| {
                    registry.highlight_all(&language, &theme, &doc)
                });
                let colors = registry.theme_colors(&theme);
                let page = html(&title, &doc.to_string(), &spans, colors, tab_width);
                drop(spans);
                // A document, which the worker saves like any other.
                let _ = tx.send(Document::new(&page));
            })
            .ok()?;
        Some(Self { path, colored, rx })
    }

    /// The page once it is made (an error if the thread stopped without
    /// one), `None` until then.
    pub fn poll(&self) -> Option<Result<Document, String>> {
        match self.rx.try_recv() {
            Ok(page) => Some(Ok(page)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("making the page stopped unexpectedly".to_owned()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLORS: ThemeColors = ThemeColors {
        foreground: [0xdd, 0xdd, 0xdd, 0xff],
        background: [0x22, 0x22, 0x22, 0xff],
    };

    fn span(start: usize, end: usize, color: [u8; 4]) -> Span {
        Span {
            start,
            end,
            color,
            bold: false,
            italic: false,
            underline: false,
        }
    }

    /// What the page's `<pre>` holds.
    fn pre(page: &str) -> &str {
        let start = page.find("<pre>\n").expect("a <pre>") + "<pre>\n".len();
        let end = page.rfind("</pre>").expect("a </pre>");
        &page[start..end]
    }

    #[test]
    fn a_page_with_the_theme_title_and_tab_width() {
        let page = html("main.rs", "fn main() {}", &[], COLORS, 4);
        assert!(page.starts_with("<!DOCTYPE html>\n"));
        assert!(page.contains("<meta charset=\"utf-8\">"));
        assert!(page.contains("<title>main.rs</title>"));
        assert!(page.contains("background: #222222;"));
        assert!(page.contains("color: #dddddd;"));
        assert!(page.contains("monospace"));
        assert!(page.contains("tab-size: 4;"));
        assert!(page.ends_with("</html>\n"));
        assert_eq!(pre(&page), "fn main() {}");
    }

    #[test]
    fn markup_characters_are_escaped_in_the_text_and_the_title() {
        let text = "if a < b && c > \"d\" {}\n<script>'x'</script>";
        let page = html("<a & \"b\">.html", text, &[], COLORS, 4);
        assert!(page.contains("<title>&lt;a &amp; &quot;b&quot;&gt;.html</title>"));
        assert_eq!(
            pre(&page),
            "if a &lt; b &amp;&amp; c &gt; &quot;d&quot; {}\n\
             &lt;script&gt;'x'&lt;/script&gt;"
        );
        // Also inside a coloured run.
        let spans: Vec<LineSpans> = vec![vec![span(5, 6, [255, 0, 0, 255])].into()];
        let page = html("t", "if a < b", &spans, COLORS, 4);
        assert_eq!(
            pre(&page),
            "if a <span style=\"color:#ff0000\">&lt;</span> b"
        );
    }

    #[test]
    fn runs_get_their_colours_and_default_ones_stay_plain() {
        let text = "fn main() {}\n// done";
        let keyword = span(0, 2, [0xc6, 0x78, 0xdd, 0xff]);
        let plain = span(2, 12, COLORS.foreground);
        let comment = Span {
            italic: true,
            bold: true,
            ..span(0, 7, [0x7f, 0x84, 0x8e, 0xff])
        };
        let spans: Vec<LineSpans> = vec![vec![keyword, plain].into(), vec![comment].into()];
        let page = html("t", text, &spans, COLORS, 4);
        assert_eq!(
            pre(&page),
            "<span style=\"color:#c678dd\">fn</span> main() {}\n\
             <span style=\"color:#7f848e;font-weight:bold;font-style:italic\">// done</span>"
        );
    }

    #[test]
    fn a_translucent_colour_is_mixed_with_the_background() {
        let spans: Vec<LineSpans> = vec![vec![span(0, 1, [255, 255, 255, 0x80])].into()];
        let page = html("t", "x", &spans, COLORS, 4);
        // Half of #ffffff over #222222.
        assert_eq!(pre(&page), "<span style=\"color:#919191\">x</span>");
    }

    #[test]
    fn an_empty_document_and_blank_lines() {
        let page = html("new 1", "", &[], COLORS, 4);
        assert_eq!(pre(&page), "");
        assert!(page.contains("<pre>\n</pre>"));
        // A blank first line survives the line break browsers drop after
        // <pre>, and lines keep their \n.
        let page = html("t", "\nx\n\ny\n", &[], COLORS, 4);
        assert!(page.contains("<pre>\n\nx\n\ny\n</pre>"));
    }

    #[test]
    fn tabs_are_kept_and_sized_by_the_tab_width() {
        let page = html("Makefile", "all:\n\tcc -o x x.c", &[], COLORS, 8);
        assert!(page.contains("tab-size: 8;"));
        assert_eq!(pre(&page), "all:\n\tcc -o x x.c");
    }

    #[test]
    fn non_ascii_text_is_kept_and_spans_count_bytes() {
        let text = "let s = \"héllo 漢字 🎉\";";
        let string = text.find('"').unwrap()..text.rfind('"').unwrap() + 1;
        let spans: Vec<LineSpans> =
            vec![vec![span(string.start, string.end, [0x98, 0xc3, 0x79, 0xff])].into()];
        let page = html("é.txt", text, &spans, COLORS, 4);
        assert!(page.contains("<title>é.txt</title>"));
        assert_eq!(
            pre(&page),
            "let s = <span style=\"color:#98c379\">&quot;héllo 漢字 🎉&quot;</span>;"
        );
        // A span cut inside a character, or past the line, is left out
        // rather than splitting it.
        let inside = text.find('é').unwrap() + 1;
        let spans: Vec<LineSpans> = vec![
            vec![
                span(inside, inside + 3, [255, 0, 0, 255]),
                span(text.len(), text.len() + 5, [255, 0, 0, 255]),
            ]
            .into(),
        ];
        let page = html("t", text, &spans, COLORS, 4);
        assert_eq!(pre(&page), "let s = &quot;héllo 漢字 🎉&quot;;");
    }

    #[test]
    fn without_spans_the_text_is_plain() {
        // A document over the highlighting limit.
        let text = "fn main() {\n    let x = 1;\n}\n";
        let page = html("big.rs", text, &[], COLORS, 4);
        assert!(!page.contains("<span"));
        assert_eq!(pre(&page), text);
        // Spans for only the first lines: the rest is plain.
        let spans: Vec<LineSpans> = vec![vec![span(0, 2, [255, 0, 0, 255])].into()];
        let page = html("big.rs", text, &spans, COLORS, 4);
        assert_eq!(
            pre(&page),
            "<span style=\"color:#ff0000\">fn</span> main() {\n    let x = 1;\n}\n"
        );
    }
}
