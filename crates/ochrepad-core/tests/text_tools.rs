//! The text tools through `Editor`: JSON format and minify, Base64 and URL
//! encoding (PLAN_v0.3.0 P4).

mod util;

use ochrepad_core::text_tools::{InvalidJson, validate_json_in};
use ochrepad_core::{Action, ActionError, ByteOffset, Codec, Indent, LineEnding};
use util::{T, setup};

fn with_indent(marked: &str, use_spaces: bool, width: usize) -> T {
    let mut t = setup(marked);
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().indent = Indent { use_spaces, width };
    t
}

#[test]
fn formatting_the_document_keeps_the_caret_on_its_token() {
    let mut t = with_indent("{\"a\":1,\"b\":[2,|3]}\n", true, 2);
    t.apply(Action::FormatJson { indent: None });
    assert_eq!(
        t.show(),
        "{\n  \"a\": 1,\n  \"b\": [\n    2,\n    |3\n  ]\n}\n",
        "indented like the document; the final line break stays"
    );
    t.apply(Action::Undo);
    assert_eq!(t.show(), "{\"a\":1,\"b\":[2,|3]}\n", "one undo step");
}

#[test]
fn a_formatted_selection_is_indented_like_its_line() {
    let mut t = with_indent("    let x = {\"a\":[1]};|", true, 4);
    t.set_sels(vec![(12, 21)]);
    t.apply(Action::FormatJson { indent: None });
    assert_eq!(
        t.show(),
        "    let x = ^{\n        \"a\": [\n            1\n        ]\n    }|;",
        "the selection covers the new text"
    );
    // A chosen indent wins over the document's.
    let mut t = with_indent("[[1]]|", false, 4);
    t.apply(Action::FormatJson {
        indent: Some(Indent {
            use_spaces: true,
            width: 2,
        }),
    });
    assert_eq!(t.text(), "[\n  [\n    1\n  ]\n]");
}

#[test]
fn minify_leaves_the_blank_space_around_the_json() {
    let mut t = setup("\n{\n\t\"a\": [ 1, 2 ],\n\t\"b\": \"x y\"\n}\n|");
    t.apply(Action::MinifyJson);
    assert_eq!(t.show(), "\n{\"a\":[1,2],\"b\":\"x y\"}\n|");
}

#[test]
fn invalid_json_is_an_error_with_its_line_and_column() {
    let mut t = setup("{\n\t\"a\" 1\n}|");
    let r = t.ed.apply(t.view, Action::FormatJson { indent: None });
    // The tab is 4 columns wide, as in the status bar.
    assert_eq!(
        r,
        Err(ActionError::InvalidJson(Box::new(InvalidJson {
            at: ByteOffset(7),
            line: 2,
            column: 9,
            message: "expected ':' after the key but found '1'".to_owned(),
        })))
    );
    assert_eq!(t.show(), "{\n\t\"a\" 1\n}|", "nothing changes");
    assert_eq!(
        r.unwrap_err().to_string(),
        "invalid JSON on line 2, column 9: expected ':' after the key but found '1'"
    );
    // Validate finds the same place, inside a selection too.
    t.set_sels(vec![(2, 9)]);
    let e = validate_json_in(t.buffer().doc(), &t.sels(), 4).unwrap_err();
    assert!(matches!(e, ActionError::InvalidJson(j) if j.at == ByteOffset(7)));
    t.set_sels(vec![(3, 6)]);
    assert_eq!(validate_json_in(t.buffer().doc(), &t.sels(), 4), Ok(()));
}

#[test]
fn every_selection_is_encoded_on_its_own_in_one_step() {
    let mut t = setup("foo bar|");
    t.set_sels(vec![(0, 3), (7, 4)]);
    t.apply(Action::Encode(Codec::Base64));
    assert_eq!(t.show(), "^Zm9v| |YmFy^", "selections keep their direction");
    t.apply(Action::Decode(Codec::Base64));
    assert_eq!(t.text(), "foo bar");
    t.apply(Action::Undo).apply(Action::Undo);
    assert_eq!(t.text(), "foo bar");
    t.set_sels(vec![(0, 7)]);
    t.apply(Action::Encode(Codec::Url));
    assert_eq!(t.text(), "foo%20bar");
}

#[test]
fn without_a_selection_the_document_is_encoded_minus_its_final_line_break() {
    let mut t = setup("hello|\n");
    t.apply(Action::Encode(Codec::Base64));
    assert_eq!(t.text(), "aGVsbG8=\n");
    t.apply(Action::Decode(Codec::Base64));
    assert_eq!(t.text(), "hello\n");
}

#[test]
fn encoding_uses_the_documents_line_endings_and_decoding_the_editors() {
    let mut t = setup("a\nb|");
    let b = t.ed.view(t.view).unwrap().buffer();
    t.ed.buffer_mut(b).unwrap().meta.line_ending = LineEnding::CrLf;
    t.apply(Action::Encode(Codec::Base64));
    assert_eq!(t.text(), "YQ0KYg==", "a CR LF line break");
    t.apply(Action::Decode(Codec::Base64));
    assert_eq!(t.text(), "a\nb");
}

#[test]
fn text_that_does_not_decode_is_refused() {
    let mut t = setup("%FF|");
    t.set_sels(vec![(0, 3)]);
    assert_eq!(
        t.ed.apply(t.view, Action::Decode(Codec::Url)),
        Err(ActionError::Decode(
            "the selection is not valid URL encoding: the decoded bytes are not UTF-8 text"
                .to_owned()
        ))
    );
    let mut t = setup("not base64!|");
    assert_eq!(
        t.ed.apply(t.view, Action::Decode(Codec::Base64)),
        Err(ActionError::Decode(
            "the document is not valid Base64: '!' is not a Base64 character".to_owned()
        ))
    );
    assert_eq!(t.text(), "not base64!");
}

#[test]
fn the_actions_read_from_json_as_documented() {
    let read = |s: &str| serde_json::from_str::<Action>(s).unwrap();
    assert_eq!(
        read(r#"{"FormatJson": {}}"#),
        Action::FormatJson { indent: None }
    );
    assert_eq!(
        read(r#"{"FormatJson": {"indent": {"use_spaces": true, "width": 2}}}"#),
        Action::FormatJson {
            indent: Some(Indent {
                use_spaces: true,
                width: 2
            })
        }
    );
    assert_eq!(read(r#""MinifyJson""#), Action::MinifyJson);
    assert_eq!(
        read(r#"{"Encode": "Base64"}"#),
        Action::Encode(Codec::Base64)
    );
    assert_eq!(read(r#"{"Decode": "Url"}"#), Action::Decode(Codec::Url));
}
