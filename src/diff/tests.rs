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
    assert_eq!(file.patch, input);
    assert_eq!((file.lines[2].old, file.lines[2].new), (Some(9), None));
    assert_eq!((file.lines[5].old, file.lines[5].new), (Some(10), Some(11)));
    assert_eq!(
        file.pairs,
        [
            Pair::Full(0),
            Pair::Lines(Some(1), Some(1)),
            Pair::Lines(Some(2), Some(3)),
            Pair::Lines(None, Some(4)),
            Pair::Lines(Some(5), Some(5))
        ]
    );
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
