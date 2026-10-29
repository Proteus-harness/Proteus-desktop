use serde_json::json;

use super::*;

#[test]
fn known_tools_name_the_action_and_its_subject() {
    let command = headline(
        "exec_command",
        &json!({"cmd": "ls -la", "max_output_tokens": 400}),
    );
    assert_eq!(command.label, "exec_command");
    assert_eq!(command.subject.as_deref(), Some("ls -la"));
    assert_eq!(
        other_args(
            &json!({"workdir": "/w", "cmd": "ls", "max_output_tokens": 400, "skip": null, "justification": "j"}),
            command.subject_key.as_deref()
        ),
        vec![
            ("justification".to_owned(), "j".to_owned()),
            ("workdir".to_owned(), "/w".to_owned())
        ]
    );
    let files = headline(
        "read_many_files",
        &json!({"paths": ["a.rs", "b.rs", "c.rs"]}),
    );
    assert_eq!(files.subject.as_deref(), Some("a.rs +2"));
    assert_eq!(headline("git_status", &json!({})).subject, None);
}

#[test]
fn unknown_tools_keep_their_name_and_find_a_conventional_subject() {
    let hover = headline("lsp_hover", &json!({"line": 3, "file_path": "src/lib.rs"}));
    assert_eq!(hover.label, "lsp_hover");
    assert_eq!(hover.subject.as_deref(), Some("src/lib.rs"));
    let note = headline("memory_store", &json!({"note": "Use nextest", "ttl": 3}));
    assert_eq!(note.subject.as_deref(), Some("Use nextest"));
    assert_eq!(headline("probe", &json!({"body": "a\nb"})).subject, None);
    let multi = headline("exec_command", &json!({"cmd": "cat <<EOF\nhi\nEOF"}));
    assert_eq!(multi.subject.as_deref(), Some("cat <<EOF ↵ hi ↵ EOF"));
}

#[test]
fn patches_list_files_with_line_stats() {
    let patch = "\
*** Begin Patch
*** Add File: a.txt
+one
+two
*** Update File: src/lib.rs
*** Move to: src/main.rs
@@
-old
+new
 context
*** End Patch";
    let files = patch_files(patch);
    assert_eq!(files.len(), 2);
    assert_eq!(
        (files[0].operation, files[0].additions, files[0].deletions),
        (PatchOperation::Add, 2, 0)
    );
    assert_eq!(files[0].body, "+one\n+two");
    assert_eq!(files[1].path, "src/lib.rs → src/main.rs");
    assert_eq!(files[1].operation, PatchOperation::Move);
    assert_eq!(files[1].body, "@@\n-old\n+new\n context");

    let summary = headline(APPLY_PATCH, &json!({ "input": patch }));
    assert_eq!(summary.subject.as_deref(), Some("a.txt +1"));
    assert_eq!(summary.stats, Some((3, 1)));
}

#[test]
fn compact_patch_stats_preserve_permissive_envelopes() {
    let cases = [
        ("", None),
        (" \r\n", None),
        ("+orphan\n-orphan\n*** Move to: ignored", None),
        ("*** Begin Patch\n*** End Patch", None),
        ("*** Add File:\n+missing header space", None),
        (" *** Add File: indented\n+not a file", None),
        ("*** Add File: ", Some((0, 0))),
        ("*** Delete File: gone\n-removed", Some((0, 1))),
        (
            "*** Update File: x\n+one\n*** End Patch\n-two\n*** Begin Patch\n+three",
            Some((2, 1)),
        ),
        (
            "*** Add File: x\n+++ header\n--- header\n@@ +hunk -hunk\n context",
            Some((1, 1)),
        ),
        (
            "*** Update File: файл.rs\r\n*** Move to: +новый.rs\r\n+🦀\r\n-старый\r\n",
            Some((1, 1)),
        ),
        (
            "*** Add File: a\n*** Move to: b\n*** Move to: c\n+one\n*** Delete File: a\n-two",
            Some((1, 1)),
        ),
    ];
    for (patch, expected) in cases {
        assert_eq!(patch_line_stats(patch), expected, "{patch:?}");
        let files = patch_files(patch);
        assert_eq!(
            (!files.is_empty()).then(|| patch_stats(&files)),
            expected,
            "{patch:?}"
        );
    }
}

#[test]
fn compact_patch_stats_match_full_parser_across_line_orderings() {
    let lines = [
        "*** Begin Patch",
        "*** End Patch",
        "*** Add File: ",
        "*** Update File: файл.rs",
        "*** Delete File: gone",
        "*** Move to: next",
        "*** Move to: ",
        "*** Add File:",
        "+added",
        "-removed",
        "@@ -1 +1 @@",
        " context",
        "",
    ];
    for first in lines {
        for second in lines {
            for third in lines {
                let patch = format!("{first}\n{second}\n{third}");
                let files = patch_files(&patch);
                let expected = (!files.is_empty()).then(|| patch_stats(&files));
                assert_eq!(patch_line_stats(&patch), expected, "{patch:?}");
            }
        }
    }
}

#[test]
fn compact_patch_stats_keep_headline_argument_precedence() {
    let patch = "*** Add File: a\n+one";
    for args in [
        json!({"patch": patch}),
        json!({"input": patch}),
        json!({"patch": "*** Delete File: b\n-two", "input": patch}),
        json!({"patch": null, "input": patch}),
        json!({"patch": 7, "input": patch}),
        json!({"patch": " \n", "input": patch}),
        json!({"patch": "+orphan"}),
        json!({}),
        json!(null),
    ] {
        assert_eq!(
            patch_text(&args).and_then(patch_line_stats),
            headline(APPLY_PATCH, &args).stats,
            "{args}"
        );
    }
}

#[test]
fn shared_patch_scan_preserves_bodies_and_repeated_moves() {
    let patch = concat!(
        "*** Move to: ignored\n*** Update File: \n\n",
        "*** Move to: новый.rs\n*** Move to: final.rs\n",
        "@@\n+🦀\n*** End Patch\n-old\n\n context\n",
        "*** End Patch \n*** Delete File: gone"
    );
    let files = patch_files(patch);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path, " → новый.rs → final.rs");
    assert_eq!(files[0].operation, PatchOperation::Move);
    assert_eq!(files[0].body, "@@\n+🦀\n-old\n\n context\n*** End Patch ");
    assert_eq!(files[1].operation, PatchOperation::Delete);
    assert_eq!(files[1].body, "");
    let summary = headline(APPLY_PATCH, &json!({"patch": patch}));
    assert_eq!(
        summary.subject.as_deref(),
        Some(" → новый.rs → final.rs +1")
    );
    assert_eq!(summary.stats, Some((1, 1)));
}

#[test]
fn compact_patch_stats_count_long_lines_and_fresh_inputs() {
    let body = "🦀".repeat(65_536);
    let patch = format!("*** Add File: wide\n+{body}\n-{body}");
    assert_eq!(patch_line_stats(&patch), Some((1, 1)));
    assert_eq!(patch_line_stats("*** Add File: wide\n+new"), Some((1, 0)));
    assert_eq!(patch_line_stats("*** Delete File: wide"), Some((0, 0)));
}

#[test]
fn plans_summarize_progress() {
    let args = json!({"plan": [
        {"step": "Read", "status": "completed"},
        {"step": "Write", "status": "in_progress"},
        {"step": "Test", "status": "pending"}
    ]});
    assert_eq!(
        headline(UPDATE_PLAN, &args).subject.as_deref(),
        Some("1/3 · Write")
    );
    assert_eq!(plan_steps(&args)[2].status, StepStatus::Pending);
}
