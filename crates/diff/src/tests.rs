use super::*;

#[test]
fn aligns_replacements_without_losing_unified_order() {
    let input = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -8,3 +8,4 @@ fn main()\n same\n-old\n+new\n+extra\n tail\n";
    let files = parse(input, &[]);
    let file = &files[0];
    assert_eq!(
        (file.path.as_str(), file.added, file.removed),
        ("src/a.rs", 2, 1)
    );
    assert_eq!(file.patch.as_str(), input);
    assert_eq!((file.lines[2].old, file.lines[2].new), (Some(9), None));
    assert_eq!((file.lines[5].old, file.lines[5].new), (Some(10), Some(11)));
    assert_eq!(
        file.pairs,
        [
            Pair::Lines(Some(1), Some(1)),
            Pair::Lines(Some(2), Some(3)),
            Pair::Lines(None, Some(4)),
            Pair::Lines(Some(5), Some(5))
        ]
    );
    assert_eq!(file.unified, [1, 2, 3, 4, 5]);
}

#[test]
fn apply_patch_coordinates_are_unknown_and_moves_keep_both_paths() {
    let files = parse(
        "*** Begin Patch\n*** Update File: old.rs\n*** Move to: src/new.rs\n@@ fn main\n-a\n+b\n*** Add File: fresh.rs\n+first\n*** Delete File: gone.rs\n*** End Patch",
        &[],
    );
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].path, "src/new.rs");
    assert_eq!(files[0].old_path.as_deref(), Some("old.rs"));
    assert_eq!((files[0].lines[1].old, files[0].lines[2].new), (None, None));
    assert_eq!(files[1].lines[0].new, Some(1));
    assert_eq!(files[2].status, "deleted");
}

#[test]
fn new_deleted_and_multiple_hunks_number_source_lines() {
    let files = parse(
        "diff --git a/a b/a\nnew file mode 100644\n--- /dev/null\n+++ b/a\n@@ -0,0 +1 @@\n+first\n\\ No newline at end of file\ndiff --git a/b b/b\ndeleted file mode 100644\n--- a/b\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\ndiff --git a/c b/c\n--- a/c\n+++ b/c\n@@ -10 +12 @@\n-x\n+y\n@@ -90 +92 @@\n-z\n+w\n",
        &[],
    );
    assert_eq!(files.len(), 3);
    assert_eq!(files[0].status, "added");
    assert_eq!(files[0].lines[1].new, Some(1));
    assert_eq!((files[0].added, files[0].removed), (1, 0));
    assert_eq!(files[1].path, "b");
    assert_eq!(files[1].status, "deleted");
    assert_eq!(files[2].lines[4].old, Some(90));
    assert_eq!(files[2].lines[5].new, Some(92));
}

#[test]
fn decodes_git_octal_utf8_and_space_paths() {
    let files = parse(
        "diff --git \"a/\\320\\260.txt\" \"b/\\320\\260.txt\"\n--- \"a/\\320\\260.txt\"\n+++ \"b/\\320\\260.txt\"\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/a b.txt b/a b.txt\nBinary files a/a b.txt and b/a b.txt differ\n",
        &[],
    );
    assert_eq!(files[0].path, "а.txt");
    assert_eq!(files[1].path, "a b.txt");
    assert_eq!(files[1].lines[0].text, "Двоичный файл");
}

#[test]
fn header_only_diffs_keep_the_destination_without_inventing_a_rename() {
    for (header, expected) in [
        (r#"diff --git "a/old\"name" "b/\320\260.txt""#, "а.txt"),
        ("diff --git a/old name b/./new name", "./new name"),
        ("diff --git a/lone", "lone"),
    ] {
        let files = parse(header, &[]);
        assert_eq!(files[0].path, expected);
        assert_eq!(files[0].old_path, None);
    }
}

#[test]
fn plain_headers_keep_distinct_paths_without_an_envelope() {
    let files = parse("--- a/old name\n+++ b/new name\n", &[]);
    assert_eq!(files[0].path, "new name");
    assert_eq!(files[0].old_path.as_deref(), Some("old name"));
}

#[test]
fn binary_oversized_and_empty_files_remain_visible() {
    let info = [
        FileInfo {
            path: "picture.png".into(),
            status: "added".into(),
            message: Some("Двоичный файл".into()),
        },
        FileInfo {
            path: "huge.txt".into(),
            message: Some("Diff превышает предел 512 КБ".into()),
            ..FileInfo::default()
        },
    ];
    let files = parse("", &info);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].lines[0].text, "Двоичный файл");
    assert_eq!(files[1].lines[0].text, "Diff превышает предел 512 КБ");
    assert!(parse("", &[]).is_empty());
}

#[test]
fn repeated_metadata_keeps_first_card_and_note_but_last_nonempty_status() {
    let patch = "diff --git a/a b/a\ndiff --git a/a b/a\n";
    let meta = |path: &str, status: &str, message: Option<&str>| FileInfo {
        path: path.into(),
        status: status.into(),
        message: message.map(str::to_owned),
    };
    for extra in [0, 256] {
        let mut info = vec![meta("a", "added", Some("first")), meta("z", "", None)];
        info.extend((0..extra).map(|i| meta(&format!("other-{i}"), "", None)));
        info.extend([
            meta("z", "conflict", Some("second")),
            meta("a", "modified", Some("ignored")),
            meta("a", "", None),
        ]);
        let files = parse(patch, &info);
        assert_eq!(files.len(), 3 + extra);
        assert_eq!(files[0].status, "modified");
        assert_eq!(files[0].lines[0].text, "first");
        assert!(files[1].status.is_empty());
        assert_eq!(files[2].path, "z");
        assert_eq!(files[2].status, "conflict");
        assert_eq!(files[2].lines[0].text, "second");
    }
}

#[test]
fn metadata_merges_without_duplicating_files() {
    let files = parse(
        "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n",
        &[FileInfo {
            path: "a".into(),
            status: "conflict".into(),
            message: None,
        }],
    );
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].status, "conflict");
}

#[test]
fn missing_newline_metadata_does_not_split_a_replacement() {
    let input = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n";
    let files = parse(input, &[]);
    let file = &files[0];
    assert_eq!(file.patch.as_str(), input);
    assert_eq!(file.unified, [1, 2]);
    assert_eq!(file.pairs, [Pair::Lines(Some(1), Some(2))]);
    assert!(file.lines[1].no_newline);
    assert!(file.lines[2].no_newline);
    assert_eq!((file.lines[1].old, file.lines[2].new), (Some(1), Some(1)));
}

#[test]
fn plain_unified_files_are_separate_and_header_like_source_is_content() {
    let input = "--- a/first.txt\n+++ b/first.txt\n@@ -1,2 +1,2 @@\n--- source\n+++ source\n-old\n+new\n--- a/second.txt\n+++ b/second.txt\n@@ -8 +9 @@\n-before\n+after\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-deleted\n";
    let files = parse(input, &[]);
    assert_eq!(files.len(), 4);
    assert_eq!(
        files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        ["first.txt", "second.txt", "new.txt", "gone.txt"]
    );
    assert_eq!(files[0].lines[1].text, "-- source");
    assert_eq!(files[0].lines[2].text, "++ source");
    assert_eq!((files[0].added, files[0].removed), (2, 2));
    assert_eq!(
        (files[1].lines[1].old, files[1].lines[2].new),
        (Some(8), Some(9))
    );
    assert_eq!(files[2].status, "added");
    assert_eq!(files[3].status, "deleted");
    assert_eq!(
        files.iter().map(|f| f.patch.as_str()).collect::<String>(),
        input
    );
}

#[test]
fn metadata_without_a_patch_does_not_claim_to_know_the_contents() {
    let raw = "diff --git a/empty.txt b/empty.txt\nnew file mode 100644\n";
    for status in ["added", "untracked", "modified", "renamed", "conflict", ""] {
        let info = FileInfo {
            path: "other.txt".into(),
            status: status.into(),
            message: None,
        };
        let files = parse(raw, &[info]);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].lines[0].text, "Пустой файл");
        assert_eq!(files[0].patch.as_str(), raw);
        let card = &files[1];
        assert_eq!(
            card.lines[0].text, "Текст изменений не предоставлен",
            "{status:?}"
        );
        assert_eq!(card.status, status);
        assert_eq!(card.unified, [0]);
        assert_eq!(card.pairs, [Pair::Full(0)]);
        assert!(card.patch.is_empty());
    }
}

#[test]
fn git_control_escapes_merge_file_metadata() {
    for (escape, decoded) in [
        ("a", '\u{7}'),
        ("b", '\u{8}'),
        ("v", '\u{b}'),
        ("f", '\u{c}'),
    ] {
        let expected = format!("name{decoded}.txt");
        let patch = format!(
            "diff --git \"a/name\\{escape}.txt\" \"b/name\\{escape}.txt\"\n--- \"a/name\\{escape}.txt\"\n+++ \"b/name\\{escape}.txt\"\n@@ -1 +1 @@\n-old\n+new\n"
        );
        let files = parse(
            &patch,
            &[FileInfo {
                path: expected.clone(),
                status: "modified".into(),
                message: None,
            }],
        );
        assert_eq!(files.len(), 1, "escape {escape}: duplicate file card");
        assert_eq!(files[0].path, expected);
        assert_eq!(files[0].patch.as_str(), patch);
        assert_eq!((files[0].added, files[0].removed), (1, 1));
    }
}

#[test]
fn git_binary_body_is_copyable_without_becoming_display_rows() {
    // Git's literal patch for adding a two-by-two PNG.
    let image = r#"diff --git a/after/tile.png b/after/tile.png
new file mode 100644
index 0000000000000000000000000000000000000000..26abf04982d6d6afd6f8a8e34d2f2b3672ffbd08
GIT binary patch
literal 73
zcmeAS@N?(olHy`uVBq!ia0vp^Od!kwBL7~QRScvAJY5_^D&{2bxGBx=!Xv=NVBEwm
UnkiN?7bwTz>FVdQ&MBb@0RKb}>i_@%

literal 0
HcmV?d00001

"#;
    let text = "diff --git a/name b/name\n--- a/name\n+++ b/name\n@@ -1 +1 @@\n-old\n+new\n";
    let files = parse(&format!("{image}{text}"), &[]);
    assert_eq!(files.len(), 2);
    let file = &files[0];
    assert_eq!(file.path, "after/tile.png");
    assert_eq!(file.status, "added");
    assert_eq!(file.patch.as_str(), image);
    assert_eq!((file.added, file.removed), (0, 0));
    assert_eq!(file.unified, [0]);
    assert_eq!(file.pairs, [Pair::Full(0)]);
    assert_eq!(file.lines[0].kind, Kind::Note);
    assert_eq!(file.lines[0].text, "Двоичный файл");
    assert_eq!(files[1].patch.as_str(), text);
    assert_eq!((files[1].added, files[1].removed), (1, 1));

    // A finished binary record must not hide unrelated annotations.
    let footer = "-- \n2.52.0\n";
    let with_footer = format!("{image}{footer}");
    let file = &parse(&with_footer, &[])[0];
    assert_eq!(file.patch.as_str(), with_footer);
    assert_eq!(
        file.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>(),
        ["Двоичный файл", "-- ", "2.52.0"]
    );
}

#[test]
fn interleaved_edits_pair_in_order_without_crossing_run_boundaries() {
    let patch =
        "*** Update File: a\n+a\n+b\n-x\n+c\n-y\n context\n-old\n@@ next\n+new\nnotice\n-other\n";
    let file = &parse(patch, &[])[0];
    assert_eq!(
        file.pairs,
        [
            Pair::Lines(Some(2), Some(0)),
            Pair::Lines(Some(4), Some(1)),
            Pair::Lines(None, Some(3)),
            Pair::Lines(Some(5), Some(5)),
            Pair::Lines(Some(6), None),
            Pair::Lines(None, Some(8)),
            Pair::Full(9),
            Pair::Lines(Some(10), None),
        ]
    );
}
