use helix_core::{FoldKind, FoldRange, Selection};
use helix_term::application::Application;
use helix_view::{current, current_ref, doc};
use std::io::Write;

use super::*;
use helix_core::comment::{
    format_comment_box, CommentBoxAlignment, CommentBoxStyle, DEFAULT_COMMENT_BOX_WIDTH,
};

fn folded_comment_app() -> anyhow::Result<Application> {
    AppBuilder::new()
        .with_input_text("above\n#[|]#/// one\n/// two\n/// three\nfn foo() {}\n")
        .build()
}

fn install_comment_fold(app: &mut Application, cursor_line: usize) {
    let (view, doc) = current!(app.editor);
    let text = doc.text().slice(..);
    let fold_start = text.line_to_char(2).saturating_sub(1);
    let fold_end = text.line_to_char(4);
    let fold =
        FoldRange::new(1, 3, fold_start, fold_end, " ⋯ 3 lines").with_kind(FoldKind::Comment);
    let cursor = text.line_to_char(cursor_line);
    doc.set_folds(vec![fold]);
    doc.set_selection(view.id, Selection::point(cursor));
}

fn cursor_line(app: &Application) -> usize {
    let (view, doc) = current_ref!(app.editor);
    let text = doc.text().slice(..);
    let cursor = doc.selection(view.id).primary().cursor(text);
    text.char_to_line(cursor)
}

mod insert;
mod movement;
mod reverse_selection_contents;
mod rotate_selection_contents;
mod write;

#[tokio::test(flavor = "multi_thread")]
async fn fold_close_and_open_all_from_keymap() -> anyhow::Result<()> {
    let mut file = tempfile::Builder::new().suffix(".lua").tempfile()?;
    write!(
        file,
        "function foo()\n  print(1)\nend\n\nfunction bar()\n  print(2)\nend\n"
    )?;

    let mut app = AppBuilder::new().with_file(file.path(), None).build()?;
    let assert_closed = |app: &Application| {
        let doc = doc!(app.editor);
        assert_eq!(doc.folds().len(), 2);
    };
    let assert_open = |app: &Application| {
        let doc = doc!(app.editor);
        assert!(doc.folds().is_empty());
    };

    test_key_sequences(
        &mut app,
        vec![
            (Some("zM"), Some(&assert_closed as &dyn Fn(&Application))),
            (Some("zR"), Some(&assert_open as &dyn Fn(&Application))),
        ],
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn vertical_movement_treats_folded_region_as_one_line() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .with_input_text("above\n/// one\n/// two\n/// three\n#[|]#fn foo() {}\n")
        .build()?;
    {
        let (view, doc) = current!(app.editor);
        let (fold_start, fold_end, function_start) = {
            let text = doc.text().slice(..);
            (
                text.line_to_char(2).saturating_sub(1),
                text.line_to_char(4),
                text.line_to_char(4),
            )
        };
        let fold =
            FoldRange::new(1, 3, fold_start, fold_end, " ⋯ 3 lines").with_kind(FoldKind::Comment);
        doc.set_folds(vec![fold]);
        doc.set_selection(view.id, Selection::point(function_start));
    }

    let assert_on_fold_line = |app: &Application| {
        let (view, doc) = current_ref!(app.editor);
        let text = doc.text().slice(..);
        let cursor = doc.selection(view.id).primary().cursor(text);
        assert_eq!(text.char_to_line(cursor), 1);
    };
    let assert_above_fold = |app: &Application| {
        let (view, doc) = current_ref!(app.editor);
        let text = doc.text().slice(..);
        let cursor = doc.selection(view.id).primary().cursor(text);
        assert_eq!(text.char_to_line(cursor), 0);
    };
    let assert_on_function_line = |app: &Application| {
        let (view, doc) = current_ref!(app.editor);
        let text = doc.text().slice(..);
        let cursor = doc.selection(view.id).primary().cursor(text);
        assert_eq!(text.char_to_line(cursor), 4);
    };

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("k"),
                Some(&assert_on_fold_line as &dyn Fn(&Application)),
            ),
            (Some("k"), Some(&assert_above_fold as &dyn Fn(&Application))),
            (
                Some("j"),
                Some(&assert_on_fold_line as &dyn Fn(&Application)),
            ),
            (
                Some("j"),
                Some(&assert_on_function_line as &dyn Fn(&Application)),
            ),
        ],
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn vertical_movement_treats_merged_folds_as_one_region() -> anyhow::Result<()> {
    let mut app = folded_comment_app()?;
    {
        let (view, doc) = current!(app.editor);
        let (first_start, first_end, second_end, function_start) = {
            let text = doc.text().slice(..);
            (
                text.line_to_char(2).saturating_sub(1),
                text.line_to_char(3),
                text.line_to_char(4),
                text.line_to_char(4),
            )
        };
        let first =
            FoldRange::new(1, 2, first_start, first_end, " ⋯ 1 lines").with_kind(FoldKind::Comment);
        let second =
            FoldRange::new(3, 4, first_end, second_end, " ⋯ 1 lines").with_kind(FoldKind::Comment);
        doc.set_folds(vec![first, second]);
        doc.set_selection(view.id, Selection::point(function_start));
        assert_eq!(doc.folds().len(), 1);
    }

    let assert_on_fold_line = |app: &Application| {
        let doc = doc!(app.editor);
        assert_eq!(doc.folds().len(), 1);
        assert_eq!(cursor_line(app), 1);
    };
    let assert_above_fold = |app: &Application| {
        let doc = doc!(app.editor);
        assert_eq!(doc.folds().len(), 1);
        assert_eq!(cursor_line(app), 0);
    };

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("k"),
                Some(&assert_on_fold_line as &dyn Fn(&Application)),
            ),
            (Some("k"), Some(&assert_above_fold as &dyn Fn(&Application))),
        ],
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn vertical_movement_does_not_enter_fold_at_end_of_file() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .with_input_text("above\n#[|]#/// one\n/// two\n/// three\n")
        .build()?;
    {
        let (view, doc) = current!(app.editor);
        let (fold_start, fold_end, cursor) = {
            let text = doc.text().slice(..);
            (
                text.line_to_char(2).saturating_sub(1),
                text.len_chars(),
                text.line_to_char(1),
            )
        };
        let fold =
            FoldRange::new(1, 4, fold_start, fold_end, " ⋯ 3 lines").with_kind(FoldKind::Comment);
        doc.set_folds(vec![fold]);
        doc.set_selection(view.id, Selection::point(cursor));
    }

    let assert_on_fold_line = |app: &Application| {
        let doc = doc!(app.editor);
        assert_eq!(doc.folds().len(), 1);
        assert_eq!(cursor_line(app), 1);
    };

    test_key_sequence(
        &mut app,
        Some("j"),
        Some(&assert_on_fold_line as &dyn Fn(&Application)),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn horizontal_movement_opens_folded_region_with_l() -> anyhow::Result<()> {
    let mut app = folded_comment_app()?;
    install_comment_fold(&mut app, 1);

    let assert_opened = |app: &Application| {
        let doc = doc!(app.editor);
        assert!(doc.folds().is_empty());
        assert_eq!(cursor_line(app), 1);
    };

    test_key_sequence(
        &mut app,
        Some("l"),
        Some(&assert_opened as &dyn Fn(&Application)),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn horizontal_movement_opens_folded_region_with_h() -> anyhow::Result<()> {
    let mut app = folded_comment_app()?;
    install_comment_fold(&mut app, 1);

    let assert_opened = |app: &Application| {
        let doc = doc!(app.editor);
        assert!(doc.folds().is_empty());
        assert_eq!(cursor_line(app), 1);
    };

    test_key_sequence(
        &mut app,
        Some("h"),
        Some(&assert_opened as &dyn Fn(&Application)),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn insert_mode_opens_fold_and_places_cursor_at_fold_start() -> anyhow::Result<()> {
    let mut app = folded_comment_app()?;
    install_comment_fold(&mut app, 1);

    let assert_insert_inside_fold = |app: &Application| {
        let doc = doc!(app.editor);
        assert!(doc.folds().is_empty());
        assert_eq!(app.editor.mode(), helix_view::document::Mode::Insert);
        assert_eq!(cursor_line(app), 2);
    };

    test_key_sequence(
        &mut app,
        Some("i"),
        Some(&assert_insert_inside_fold as &dyn Fn(&Application)),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn append_mode_does_not_open_folded_region() -> anyhow::Result<()> {
    let mut app = folded_comment_app()?;
    install_comment_fold(&mut app, 1);

    let assert_still_folded = |app: &Application| {
        let doc = doc!(app.editor);
        assert_eq!(doc.folds().len(), 1);
    };

    test_key_sequence(
        &mut app,
        Some("a"),
        Some(&assert_still_folded as &dyn Fn(&Application)),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn open_below_inserts_after_folded_region_without_opening() -> anyhow::Result<()> {
    let mut app = folded_comment_app()?;
    install_comment_fold(&mut app, 1);

    let assert_line_after_fold = |app: &Application| {
        let doc = doc!(app.editor);
        assert_eq!(doc.folds().len(), 1);
        assert_eq!(doc.folds()[0].start_line, 1);
        assert_eq!(doc.folds()[0].end_line, 3);
        assert_eq!(cursor_line(app), 4);
    };

    test_key_sequence(
        &mut app,
        Some("o"),
        Some(&assert_line_after_fold as &dyn Fn(&Application)),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn open_above_inserts_before_folded_region_without_opening() -> anyhow::Result<()> {
    let mut app = folded_comment_app()?;
    install_comment_fold(&mut app, 1);

    let assert_line_before_fold = |app: &Application| {
        let doc = doc!(app.editor);
        assert_eq!(doc.folds().len(), 1);
        assert_eq!(doc.folds()[0].start_line, 2);
        assert_eq!(doc.folds()[0].end_line, 4);
        assert_eq!(cursor_line(app), 1);
    };

    test_key_sequence(
        &mut app,
        Some("O"),
        Some(&assert_line_before_fold as &dyn Fn(&Application)),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn search_selection_detect_word_boundaries_at_eof() -> anyhow::Result<()> {
    // <https://github.com/helix-editor/helix/issues/12609>
    test((
        indoc! {"\
            #[o|]#ne
            two
            three"},
        "gej*h",
        indoc! {"\
            one
            two
            three#[
            |]#"},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_selection_duplication() -> anyhow::Result<()> {
    // Forward
    test((
        indoc! {"\
            #[lo|]#rem
            ipsum
            dolor
            "},
        "CC",
        indoc! {"\
            #(lo|)#rem
            #(ip|)#sum
            #[do|]#lor
            "},
    ))
    .await?;

    // Backward
    test((
        indoc! {"\
            #[|lo]#rem
            ipsum
            dolor
            "},
        "CC",
        indoc! {"\
            #(|lo)#rem
            #(|ip)#sum
            #[|do]#lor
            "},
    ))
    .await?;

    // Copy the selection to previous line, skipping the first line in the file
    test((
        indoc! {"\
            test
            #[testitem|]#
            "},
        "<A-C>",
        indoc! {"\
            test
            #[testitem|]#
            "},
    ))
    .await?;

    // Copy the selection to previous line, including the first line in the file
    test((
        indoc! {"\
            test
            #[test|]#
            "},
        "<A-C>",
        indoc! {"\
            #[test|]#
            #(test|)#
            "},
    ))
    .await?;

    // Copy the selection to next line, skipping the last line in the file
    test((
        indoc! {"\
            #[testitem|]#
            test
            "},
        "C",
        indoc! {"\
            #[testitem|]#
            test
            "},
    ))
    .await?;

    // Copy the selection to next line, including the last line in the file
    test((
        indoc! {"\
            #[test|]#
            test
            "},
        "C",
        indoc! {"\
            #(test|)#
            #[test|]#
            "},
    ))
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_goto_file_impl() -> anyhow::Result<()> {
    let file = tempfile::NamedTempFile::new()?;

    fn match_paths(app: &Application, matches: Vec<&str>) -> usize {
        app.editor
            .documents()
            .filter_map(|d| d.path()?.file_name())
            .filter(|n| matches.iter().any(|m| *m == n.to_string_lossy()))
            .count()
    }

    // Single selection
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("ione.js<esc>%gf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one.js"]));
        }),
        false,
    )
    .await?;

    // Multiple selection
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("ione.js<ret>two.js<esc>%<A-s>gf"),
        Some(&|app| {
            assert_eq!(2, match_paths(app, vec!["one.js", "two.js"]));
        }),
        false,
    )
    .await?;

    // Cursor on first quote
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("iimport 'one.js'<esc>B;gf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one.js"]));
        }),
        false,
    )
    .await?;

    // Cursor on last quote
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("iimport 'one.js'<esc>bgf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one.js"]));
        }),
        false,
    )
    .await?;

    // ';' is behind the path
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("iimport 'one.js';<esc>B;gf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one.js"]));
        }),
        false,
    )
    .await?;

    // allow numeric values in path
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("iimport 'one123.js'<esc>B;gf"),
        Some(&|app| {
            assert_eq!(1, match_paths(app, vec!["one123.js"]));
        }),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_selection_paste() -> anyhow::Result<()> {
    test((
        indoc! {"\
            #[|lorem]#
            #(|ipsum)#
            #(|dolor)#
            "},
        "yp",
        indoc! {"\
            lorem#[|lorem]#
            ipsum#(|ipsum)#
            dolor#(|dolor)#
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_selection_shell_commands() -> anyhow::Result<()> {
    // pipe
    test((
        indoc! {"\
            #[|lorem]#
            #(|ipsum)#
            #(|dolor)#
            "},
        "|echo foo<ret>",
        indoc! {"\
            #[|foo]#
            #(|foo)#
            #(|foo)#"
        },
    ))
    .await?;

    // insert-output
    test((
        indoc! {"\
            #[|lorem]#
            #(|ipsum)#
            #(|dolor)#
            "},
        "!echo foo<ret>",
        indoc! {"\
            #[|foo]#lorem
            #(|foo)#ipsum
            #(|foo)#dolor
            "},
    ))
    .await?;

    // append-output
    test((
        indoc! {"\
            #[|lorem]#
            #(|ipsum)#
            #(|dolor)#
            "},
        "<A-!>echo foo<ret>",
        indoc! {"\
            lorem#[|foo]#
            ipsum#(|foo)#
            dolor#(|foo)#
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_undo_redo() -> anyhow::Result<()> {
    // A jumplist selection is created at a point which is undone.
    //
    // * 2[<space>   Add two newlines at line start. We're now on line 3.
    // * <C-s>       Save the selection on line 3 in the jumplist.
    // * u           Undo the two newlines. We're now on line 1.
    // * <C-o><C-i>  Jump forward an back again in the jumplist. This would panic
    //               if the jumplist were not being updated correctly.
    test((
        "#[|]#",
        "2[<space><C-s>u<C-o><C-i>",
        "#[|]#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    // A jumplist selection is passed through an edit and then an undo and then a redo.
    //
    // * [<space>    Add a newline at line start. We're now on line 2.
    // * <C-s>       Save the selection on line 2 in the jumplist.
    // * kd          Delete line 1. The jumplist selection should be adjusted to the new line 1.
    // * uU          Undo and redo the `kd` edit.
    // * <C-o>       Jump back in the jumplist. This would panic if the jumplist were not being
    //               updated correctly.
    // * <C-i>       Jump forward to line 1.
    test((
        "#[|]#",
        "[<space><C-s>kduU<C-o><C-i>",
        "#[|]#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    // In this case we 'redo' manually to ensure that the transactions are composing correctly.
    test((
        "#[|]#",
        "[<space>u[<space>u",
        "#[|]#",
        LineFeedHandling::AsIs,
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_extend_line() -> anyhow::Result<()> {
    // extend with line selected then count
    test((
        indoc! {"\
            #[l|]#orem
            ipsum
            dolor
            
            "},
        "x2x",
        indoc! {"\
            #[lorem
            ipsum
            dolor\n|]#
            
            "},
    ))
    .await?;

    // extend with count on partial selection
    test((
        indoc! {"\
            #[l|]#orem
            ipsum
            
            "},
        "2x",
        indoc! {"\
            #[lorem
            ipsum\n|]#
            
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_character_info() -> anyhow::Result<()> {
    // UTF-8, single byte
    test_key_sequence(
        &mut helpers::AppBuilder::new().build()?,
        Some("ih<esc>h:char<ret>"),
        Some(&|app| {
            assert_eq!(
                r#""h" (U+0068) Dec 104 Hex 68"#,
                app.editor.get_status().unwrap().0
            );
        }),
        false,
    )
    .await?;

    // UTF-8, multi-byte
    test_key_sequence(
        &mut helpers::AppBuilder::new().build()?,
        Some("ië<esc>h:char<ret>"),
        Some(&|app| {
            assert_eq!(
                r#""ë" (U+0065 U+0308) Hex 65 + cc 88"#,
                app.editor.get_status().unwrap().0
            );
        }),
        false,
    )
    .await?;

    // Multiple characters displayed as one, escaped characters
    test_key_sequence(
        &mut helpers::AppBuilder::new().build()?,
        Some(":line<minus>ending crlf<ret>:char<ret>"),
        Some(&|app| {
            assert_eq!(
                r#""\r\n" (U+000d U+000a) Hex 0d + 0a"#,
                app.editor.get_status().unwrap().0
            );
        }),
        false,
    )
    .await?;

    // Non-UTF-8
    test_key_sequence(
        &mut helpers::AppBuilder::new().build()?,
        Some(":encoding ascii<ret>ih<esc>h:char<ret>"),
        Some(&|app| {
            assert_eq!(r#""h" Dec 104 Hex 68"#, app.editor.get_status().unwrap().0);
        }),
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_char_backward() -> anyhow::Result<()> {
    // don't panic when deleting overlapping ranges
    test(("#(x|)# #[x|]#", "c<space><backspace><esc>", "#[\n|]#")).await?;
    test((
        "#( |)##( |)#a#( |)#axx#[x|]#a",
        "li<backspace><esc>",
        "#(a|)##(|a)#xx#[|a]#",
    ))
    .await?;

    Ok(())
}

// Cursor behavior is different when the text is created in the buffer vs loaded from a file.
// This test will not work for reproducing the crash or verifying the result after the fix.
// // #[tokio::test(flavor = "multi_thread")]
// async fn test_try_restore_indent() -> anyhow::Result<()> {
//     test((" #[ |]#foo\na#( |)#bar\n", "o<C-u><esc>", " foo\n#[\n|]#a bar\n#(\n|)#")).await?;
//     Ok(())
// }

#[tokio::test(flavor = "multi_thread")]
async fn test_try_restore_indent() -> anyhow::Result<()> {
    // Bug: 15228 try_restore_indent uses primary cursor position for all selections,
    // causing invalid range errors when multiple cursors are on different lines
    let file = temp_file_with_contents("  foo\na bar\n")?;
    test_key_sequence(
        &mut AppBuilder::new().with_file(file.path(), None).build()?,
        Some("jl<A-C>o<C-u><esc>"),
        None,
        false,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_word_backward() -> anyhow::Result<()> {
    // don't panic when deleting overlapping ranges
    test(("fo#[o|]#ba#(r|)#", "a<C-w><esc>", "#[\n|]#")).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_word_forward() -> anyhow::Result<()> {
    // don't panic when deleting overlapping ranges
    test(("fo#[o|]#b#(|ar)#", "i<A-d><esc>", "fo#[\n|]#")).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_char_forward() -> anyhow::Result<()> {
    test((
        indoc! {"\
                #[abc|]#def
                #(abc|)#ef
                #(abc|)#f
                #(abc|)#
            "},
        "a<del><esc>",
        indoc! {"\
                #[abc|]#ef
                #(abc|)#f
                #(abc|)#
                #(abc|)#
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_insert_with_indent() -> anyhow::Result<()> {
    const INPUT: &str = indoc! { "
        #[f|]#n foo() {
            if let Some(_) = None {

            }
         
        }

        fn bar() {

        }
        "
    };

    // insert_at_line_start
    test((
        INPUT,
        ":lang rust<ret>%<A-s>I",
        indoc! { "
            #[f|]#n foo() {
                #(i|)#f let Some(_) = None {
                    #(\n|)#
                #(}|)#
            #( |)#
            #(}|)#
            #(\n|)#
            #(f|)#n bar() {
                #(\n|)#
            #(}|)#
            "
        },
    ))
    .await?;

    // insert_at_line_end
    test((
        INPUT,
        ":lang rust<ret>%<A-s>A",
        indoc! { "
            fn foo() {#[\n|]#
                if let Some(_) = None {#(\n|)#
                    #(\n|)#
                }#(\n|)#
             #(\n|)#
            }#(\n|)#
            #(\n|)#
            fn bar() {#(\n|)#
                #(\n|)#
            }#(\n|)#
            "
        },
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_join_selections() -> anyhow::Result<()> {
    // normal join
    test((
        indoc! {"\
            #[a|]#bc
            def
        "},
        "J",
        indoc! {"\
            #[a|]#bc def
        "},
    ))
    .await?;

    // join with empty line
    test((
        indoc! {"\
            #[a|]#bc

            def
        "},
        "JJ",
        indoc! {"\
            #[a|]#bc def
        "},
    ))
    .await?;

    // join with additional space in non-empty line
    test((
        indoc! {"\
            #[a|]#bc

                def
        "},
        "JJ",
        indoc! {"\
            #[a|]#bc def
        "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_join_selections_space() -> anyhow::Result<()> {
    // join with empty lines panic
    test((
        indoc! {"\
            #[a

            b

            c

            d

            e|]#
        "},
        "<A-J>",
        indoc! {"\
            a#[ |]#b#( |)#c#( |)#d#( |)#e
        "},
    ))
    .await?;

    // normal join
    test((
        indoc! {"\
            #[a|]#bc
            def
        "},
        "<A-J>",
        indoc! {"\
            abc#[ |]#def
        "},
    ))
    .await?;

    // join with empty line
    test((
        indoc! {"\
            #[a|]#bc

            def
        "},
        "<A-J>",
        indoc! {"\
            #[a|]#bc
            def
        "},
    ))
    .await?;

    // join with additional space in non-empty line
    test((
        indoc! {"\
            #[a|]#bc

                def
        "},
        "<A-J><A-J>",
        indoc! {"\
            abc#[ |]#def
        "},
    ))
    .await?;

    // join with retained trailing spaces
    test((
        indoc! {"\
            #[aaa   

            bb  

            c |]#
        "},
        "<A-J>",
        indoc! {"\
            aaa   #[ |]#bb  #( |)#c 
        "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_join_selections_comment() -> anyhow::Result<()> {
    test((
        indoc! {"\
            /// #[a|]#bc
            /// def
        "},
        ":lang rust<ret>J",
        indoc! {"\
            /// #[a|]#bc def
        "},
    ))
    .await?;

    // Only join if the comment token matches the previous line.
    test((
        indoc! {"\
            #[| // a
            // b
            /// c
            /// d
            e
            /// f
            // g]#
        "},
        ":lang rust<ret>J",
        indoc! {"\
            #[| // a b /// c d e f // g]#
        "},
    ))
    .await?;

    test((
        "#[|\t// Join comments
\t// with indent]#",
        ":lang go<ret>J",
        "#[|\t// Join comments with indent]#",
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_toggle_comments_inside_comment_injection() -> anyhow::Result<()> {
    // A `//` line comment's text is injected as the `comment` language, which has no
    // comment-tokens of its own. With the cursor inside the comment, toggling must
    // resolve tokens from the enclosing language and un-comment the line,
    // not fall back to the hardcoded default `#`.
    test((
        indoc! {"\
            // #[a|]#bc
        "},
        ":lang rust<ret><C-c>",
        indoc! {"\
            #[a|]#bc
        "},
    ))
    .await?;

    // A `///` doc comment's text is injected as markdown (no line comment token of
    // its own). Toggling must strip the whole `///` marker via Rust's tokens rather
    // than insert a markdown `<!-- -->` inside or leave a stray `/`.
    test((
        indoc! {"\
            /// #[a|]#bc
        "},
        ":lang rust<ret><C-c>",
        indoc! {"\
            #[a|]#bc
        "},
    ))
    .await?;

    // Likewise for the `//!` inner doc comment marker.
    test((
        indoc! {"\
            //! #[a|]#bc
        "},
        ":lang rust<ret><C-c>",
        indoc! {"\
            #[a|]#bc
        "},
    ))
    .await?;

    // Commenting a normal code line still uses the top-level language's token
    // (no injection layer at the cursor), no regression for the common case.
    test((
        indoc! {"\
            #[l|]#et x = 5;
        "},
        ":lang rust<ret><C-c>",
        indoc! {"\
            // #[l|]#et x = 5;
        "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_read_file() -> anyhow::Result<()> {
    let mut file = tempfile::NamedTempFile::new()?;
    let contents_to_read = "some contents";
    let output_file = helpers::temp_file_with_contents(contents_to_read)?;

    test_key_sequence(
        &mut helpers::AppBuilder::new()
            .with_file(file.path(), None)
            .build()?,
        Some(&format!(":r {:?}<ret><esc>:w<ret>", output_file.path())),
        Some(&|app| {
            assert!(!app.editor.is_err(), "error: {:?}", app.editor.get_status());
        }),
        false,
    )
    .await?;

    let expected_contents = LineFeedHandling::Native.apply(contents_to_read);
    helpers::assert_file_has_content(&mut file, &expected_contents)?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn surround_delete() -> anyhow::Result<()> {
    // Test `surround_delete` when head < anchor
    test(("(#[|  ]#)", "mdm", "#[|  ]#")).await?;
    test(("(#[|  ]#)", "md(", "#[|  ]#")).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn surround_replace_ts() -> anyhow::Result<()> {
    const INPUT: &str = r#"\
fn foo() {
    if let Some(_) = None {
        testing!("f#[|o]#o)");
    }
}
"#;
    test((
        INPUT,
        ":lang rust<ret>mrm'",
        r#"\
fn foo() {
    if let Some(_) = None {
        testing!('f#[|o]#o)');
    }
}
"#,
    ))
    .await?;

    test((
        INPUT,
        ":lang rust<ret>3mrm[",
        r#"\
fn foo() {
    if let Some(_) = None [
        testing!("f#[|o]#o)");
    ]
}
"#,
    ))
    .await?;

    test((
        INPUT,
        ":lang rust<ret>2mrm{",
        r#"\
fn foo() {
    if let Some(_) = None {
        testing!{"f#[|o]#o)"};
    }
}
"#,
    ))
    .await?;

    test((
        indoc! {"\
            #[a
            b
            c
            d
            e|]#
            f
            "},
        "s\\n<ret>r,",
        "a#[,|]#b#(,|)#c#(,|)#d#(,|)#e\nf\n",
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn macro_play_within_macro_record() -> anyhow::Result<()> {
    // <https://github.com/helix-editor/helix/issues/12697>
    //
    // * `"aQihello<esc>Q` record a macro to register 'a' which inserts "hello"
    // * `Q"aq<space>world<esc>Q` record a macro to the default macro register which plays the
    //   macro in register 'a' and then inserts " world"
    // * `%d` clear the buffer
    // * `q` replay the macro in the default macro register
    // * `i<ret>` add a newline at the end
    //
    // The inner macro in register 'a' should replay within the outer macro exactly once to insert
    // "hello world".
    test((
        indoc! {"\
            #[|]#
        "},
        r#""aQihello<esc>QQ"aqi<space>world<esc>Q%dqi<ret>"#,
        indoc! {"\
            hello world
            #[|]#"},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn global_search_with_multibyte_chars() -> anyhow::Result<()> {
    // Assert that `helix_term::commands::global_search` handles multibyte characters correctly.
    test((
        indoc! {"\
            // Hello world!
            // #[|
            ]#
            "},
        // start global search
        " /«十分に長い マルチバイトキャラクター列» で検索<ret><esc>",
        indoc! {"\
            // Hello world!
            // #[|
            ]#
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn buffer_search_jumps_to_filtered_line() -> anyhow::Result<()> {
    test((
        indoc! {"\
            alpha
            #[b|]#eta
            unique needle
            gamma
            "},
        " sbneedle<ret>",
        indoc! {"\
            alpha
            beta
            #[unique needle|]#
            gamma
            "},
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_box_formats_selected_title() -> anyhow::Result<()> {
    let expected = format_comment_box(
        "//",
        CommentBoxStyle::Box,
        CommentBoxAlignment::Left,
        DEFAULT_COMMENT_BOX_WIDTH,
        &["Parser".to_string()],
    );

    test((
        "#[Parser|]#",
        ":comment-box box<ret>",
        format!("#[{expected}|]#"),
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_box_updates_existing_block() -> anyhow::Result<()> {
    let input = format_comment_box(
        "//",
        CommentBoxStyle::Box,
        CommentBoxAlignment::Left,
        DEFAULT_COMMENT_BOX_WIDTH,
        &["Parser".to_string(), "Token recovery".to_string()],
    );
    let expected = format_comment_box(
        "//",
        CommentBoxStyle::Ruler,
        CommentBoxAlignment::Center,
        DEFAULT_COMMENT_BOX_WIDTH,
        &["Parser".to_string(), "Token recovery".to_string()],
    );

    test((
        format!("#[{input}|]#"),
        ":comment-box ruler<ret>",
        format!("#[{expected}|]#"),
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_box_converts_subheading_to_box_without_filler() -> anyhow::Result<()> {
    let input = format_comment_box(
        "//",
        CommentBoxStyle::Subheading,
        CommentBoxAlignment::Left,
        DEFAULT_COMMENT_BOX_WIDTH,
        &["Pool Methods".to_string()],
    );
    let expected = format_comment_box(
        "//",
        CommentBoxStyle::Box,
        CommentBoxAlignment::Left,
        DEFAULT_COMMENT_BOX_WIDTH,
        &["Pool Methods".to_string()],
    );

    test((
        format!("#[{input}|]#"),
        ":comment-box box<ret>",
        format!("#[{expected}|]#"),
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn comment_box_preserves_selected_blank_line_after_existing_block() -> anyhow::Result<()> {
    let input = format_comment_box(
        "//",
        CommentBoxStyle::Heading,
        CommentBoxAlignment::Left,
        DEFAULT_COMMENT_BOX_WIDTH,
        &["Entity management".to_string()],
    );
    let expected = format_comment_box(
        "//",
        CommentBoxStyle::Subheading,
        CommentBoxAlignment::Left,
        DEFAULT_COMMENT_BOX_WIDTH,
        &["Entity management".to_string()],
    );

    test((
        format!("#[{input}\n\n|]#fn Entity Registry.create_entity(&self) {{\n}}"),
        ":comment-box subheading<ret>",
        format!("#[{expected}\n\n|]#fn Entity Registry.create_entity(&self) {{\n}}"),
    ))
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn align_selections_with_varying_columns() -> anyhow::Result<()> {
    test((
        indoc! {r"
            #[|]#I    I  II I
            IIIIIIIII
            IIIII
            IIIIIIIII
        "},
        r"%sI<ret>&gg",
        indoc! {r"
            #[I|]#    I  II I
            I    I  II IIIII
            I    I  II I
            I    I  II IIIII
        "},
    ))
    .await?;

    Ok(())
}
