# Proteus component patches (2026-11-07)

Upstream provenance remains in `README.proteus.md`. This ledger records the
applicable component findings from the 2026-11-07 source audit. A vendor API fix
is not evidence that Desktop exercises that API. The checks below are synthetic
GPUI/unit tests; no native interaction, RSS, latency or microphone measurements
are claimed.

| Audit area | Change and scope |
| --- | --- |
| VC01, VC02 | Code-block highlighting resolves registered canonical names before retention; shared LRU admits at most 8 parsers and 8 MiB of source, with a 2 MiB block ceiling. Larger blocks retain readable plain text. Registry generation invalidates cached parser/query state, including same-name replacement. Vendor dynamic-language/Markdown API. |
| VC03 | Foreground parsing retains valid host syntax but defers injection parsing to the existing background task. Repeated foreground calls preserve pending work. The host parser's deadline no longer admits hundreds of independent injection budgets. Desktop code-editor route exists; elapsed time was not measured. |
| VC04 | NumberInput forwards an explicitly supplied placeholder, including an explicit clear; an omitted builder leaves the state's placeholder intact. API-only. |
| VC05 | Popup activation, hover, initial navigation, wrapping and submenu entry exclude disabled/nonclickable rows. Both ordinary/custom handlers are guarded. Desktop popup route exists. |
| VC06 | AppMenu children weakly reference their bar; external owner release can release the entire graph. API-only. |
| VC07, VC08 | Deferred button callbacks close their stable dialog identity, with idempotent closure. Focus restores at logical close, without a delayed timer that can override a newer focus choice. Closing a lower modal rebases newer return-focus chains. Desktop modal route exists; the reentrant callback case is API-level. |
| VC09 | Removed the inert `Sheet::resizable` builder and unused field. Sheets expose their supported fixed `size`; this is an alpha vendor API change, not a resizing implementation. No Desktop caller found. |
| VC10–VC12 | Empty carousel selection is lazy; zero chart tick margin means every category; ColorPicker forwards its anchor. API-only. |
| VC13 | Combobox compares selected values rather than transient filtered row positions before emitting Change/Confirm and closing. API-only. |
| VC14–VC16 | Label maps folded Unicode matches back to original UTF-8 ranges. Inspector applies the first real user Change (programmatic set_value is silent). Sectioned List starts/searches at a real entry and rejects stale/nonexistent confirmation paths. Inspector is debug/optional; other controls API-only. |
| Component-tail01–03 | Retained number settings refresh bounds and setter before events. Table centralizes selectable-column/cell policy, skips excluded columns, handles empty dimensions, and recomputes grouped header geometry on moves. API-only. |
| Component-tail04 | Optional microphone capture uses an 8-entry queue, 16,384-sample callback cap and bounded drains with explicit overflow errors. Connecting speech has a 10-second deadline. macOS pending authorization audio is capped at 32,000 samples; overflow fails the session. macOS implementation is static review only on Linux. No current Desktop speech consumer established. |
| Component-tail05–06 | Hex parsing rejects non-ASCII before slicing; HSL hue uses the documented first-color weight consistently with other channels. API-only. |
| Theme1–5 | Bundled colors use canonical schema keys; legacy spellings remain deserialization aliases. Semantic selection applies to legacy/token/JSON forms. Directory defaults reconcile edits/removal against their original baseline. GroupBox title color is resolved/painted. Violet950 hex metadata now agrees with its RGB/HSL. Desktop theme loading uses canonical keys; watch_dir/semantic application are vendor APIs. |
| Overlay C1–C4 and hover lifetime | Completion/action delegates weakly reference menus. InputGroup preserves child readonly. Synchronization hides old code actions without asynchronously dismissing newer engine state. Hover snapshots include the engine revision and the popover weakly owns its editor. Find creates the menu graph even without a provider; other overlay cases remain API-only. |
| Chat caption role | Optional `chat.secondary.foreground` resolves independently from stronger settings `muted.foreground`, with a muted fallback for custom themes and a matching ThemeToken. Builtin default JSON maps it explicitly; Desktop applies it only to operational chat captions. |
| Alternate theme backend | Dotted editor keys, diagnostic StatusColors and underline decoration match the tree-sitter backend. Current Desktop uses tree-sitter. |
| Other confirmed API mismatches | Explicit notification icons override the type fallback; label_indent(false) preserves an explicit Field label. |

Rich text D1/D2/V1–V4, virtual list V5/V6, tree V7, and geometry W1–W8 are
implemented in `gpui-base` in this checkout; their actual source and regressions
are maintained there, not duplicated in component reexport facades. Selection
copy whitespace repair is likewise shared with Base.

The extraction's five test-only references to absent upstream workspace files
were made self-contained using bundled icon assets, the checked-out Base dock
source, and synthetic Markdown/theme fixtures. This does not substitute fixture
coverage for native rendering evidence.

Not promoted: cancelled tree-sitter parser resumption, ERB query identity,
JavaScript/TypeScript capture ordering, raw link release behavior, StepAfter,
malformed grid dashes, offset placement, reduced-motion loading circles and
native Windows menu refresh remain investigation/platform-test candidates.
Explicit compatibility tests for ButtonGroup/ToggleGroup/Link are preserved.
Rating accessibility, chord hints, ListItem selectable-row policy and legacy
source-copy fidelity limitations remain documented vendor limitations, not
new production defects. The unused time/utils.rs is not the live Calendar.

## Verification

Run from the Desktop root; the helper creates a standalone manifest because
vendor packages are excluded from the application workspace:

```sh
python3 scripts/test_vendor.py gpui-component --filter audit_
python3 scripts/test_vendor.py gpui-component --full
```

The audit's HTML-injection regression additionally requires
`tree-sitter-html,tree-sitter-javascript` features; the verification manifest can
be generated with `--prepare-only` and passed to Cargo with those features.
Full-suite baseline comparison and optional-backend results are recorded in the
coordinated audit verification ledger. Passing focused tests does not imply all
unrelated upstream paint/layout tests passed.

Recorded results for the coordinated 2026-11-07 checkout:

- Native tree-sitter (HTML/CSS/JavaScript/Rust): **26 focused audit tests passed**.
  Full component suite: **763 passed, 15 failed**. A separate source snapshot of
  Desktop `d655306c894938087cec9f57174a278448f5ee9b`'s component, with only the
  same five missing-fixture repairs and the same current patched Base dependency,
  produced **737 passed and exactly the same 15 failures**. No additional failure
  names appeared. These are comparative tests, not a clean upstream/full-suite
  pass: accordion (2), dock presentation (5), popover animation (1), and cross
  renderer virtual selection (7) remain unresolved in this headless test setup.
- Alternate backend without tree-sitter, plus optional Linux speech/CPAL:
  **24 focused audit tests passed**, including bounded capture callback/queue
  admission. This does not exercise physical microphone capture, platform speech
  authorization or macOS recognizer code.
- Changed Rust files pass rustfmt; `git diff --check` passes.

Local raw logs and baseline-source identity evidence are under the ignored
`build/verification/audit-20261107/component-*.log` and
`component-baseline-evidence.json`. They are verification artifacts rather than
tracked source or release evidence.
