//! JSON lines between the client and an extension process.
//!
//! The client says hello once, then asks for views and forwards user actions
//! and host events. The process answers with whole view trees whenever it
//! wants: after a request, an action or on its own timer.

use std::path::PathBuf;

pub use proteus_contracts::app_protocol::settings::SessionSettingUpdate as SessionSetting;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::presentation::Face;
use super::widget::Node;

pub const PROTOCOL: u32 = 3;
pub const FEATURES: &[&str] = &[
    "session.settings.v1",
    "widgets.controls.v1",
    "widgets.search.v1",
    "widgets.selection.v1",
    "widgets.picker.v1",
    "widgets.sortable.v1",
    "widgets.status.v1",
    "widgets.diff.v1",
    "widgets.lanes.v2",
    "widgets.disclosure.v1",
    "widgets.person.v1",
    "documents.title.v1",
    "surfaces.content.v1",
    "widgets.svg.v1",
];

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outgoing {
    CommandResult {
        id: String,
        session_dir: PathBuf,
        config: Option<Box<proteus_contracts::app_protocol::config::ConfigSummary>>,
        error: Option<String>,
    },
    SessionSnapshot {
        session_dir: Option<PathBuf>,
        state: Option<Box<super::session::State>>,
        entries: Vec<super::session::Entry>,
    },
    SessionUpdate {
        session_dir: PathBuf,
        update: super::session::Update,
    },
    /// First message: how to reach the agent server on its own.
    Hello {
        protocol: u32,
        features: Vec<String>,
        workspace: PathBuf,
        server: Server,
    },
    /// A surface was shown; the process should send its view.
    Render {
        view: String,
        surface: SurfaceKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Content>,
        #[serde(skip_serializing_if = "Option::is_none")]
        argument: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_dir: Option<PathBuf>,
    },
    /// The surface is gone; its view is no longer needed.
    Close { view: String },
    /// The user pressed a button or a list row.
    Action {
        sequence: u64,
        view: String,
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
    },
    /// Something changed in the client, e.g. another chat became active.
    Event {
        event: HostEventKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        document: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_dir: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Server {
    pub origin: String,
    pub token: String,
}

/// Original closed fence contents, separate from its stable view address.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Content {
    pub language: String,
    pub source: String,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    Page,
    Panel,
    Document,
    Badge,
    Popup,
    Content,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostEventKind {
    SessionOpened,
    RunSettled,
    NavigationChanged,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Incoming {
    /// A content renderer declined this source; the host preserves the raw fence.
    ContentError {
        view: String,
        session_dir: Option<PathBuf>,
        error: String,
        #[serde(default)]
        sequence: u64,
    },
    Focus {
        view: String,
        control: Option<String>,
    },
    ClosePopup,
    Clipboard {
        text: String,
    },
    SetSession {
        id: String,
        session_dir: PathBuf,
        setting: SessionSetting,
    },
    /// Icon/ring/text and full hover details, bound to the current session.
    Badge {
        #[serde(default)]
        hidden: bool,
        view: String,
        session_dir: Option<PathBuf>,
        title: String,
        lines: Vec<String>,
        face: Face,
    },
    /// The whole tree of one view.
    View {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        keys: std::collections::HashMap<String, String>,
        session_dir: Option<PathBuf>,
        view: String,
        root: Node,
        #[serde(default)]
        sequence: u64,
    },
    /// Opens a document of this extension as a tab.
    OpenDocument {
        argument: String,
        #[serde(default)]
        preview: bool,
        #[serde(default = "yes")]
        focus: bool,
    },
}

fn yes() -> bool {
    true
}

/// Parses one line from the process.
pub fn parse(line: &str) -> anyhow::Result<Incoming> {
    let message: Incoming = serde_json::from_str(line)?;
    if let Incoming::View {
        view,
        session_dir,
        sequence,
        root,
        ..
    } = &message
        && let Err(error) = root.validate()
    {
        // A valid envelope still identifies the failed content surface.
        // Keep its error local, just like a renderer's own content_error;
        // a bad SVG must not invalidate other views of the same process.
        if view.starts_with("content:") {
            return Ok(Incoming::ContentError {
                view: view.clone(),
                session_dir: session_dir.clone(),
                sequence: *sequence,
                error: format!("{error:#}"),
            });
        }
        return Err(error);
    }
    if let Incoming::Badge { face, .. } = &message {
        face.validate()?;
    }
    Ok(message)
}

/// View id of a surface; documents are told apart by their argument.
pub fn view_id(surface: SurfaceKind, argument: Option<&str>) -> String {
    match (surface, argument) {
        (SurfaceKind::Panel, _) => "panel".into(),
        (SurfaceKind::Page, _) => "page".into(),
        (SurfaceKind::Badge, _) => "badge".into(),
        (SurfaceKind::Popup, _) => "popup".into(),
        (SurfaceKind::Document, argument) => format!("document:{}", argument.unwrap_or("")),
        (SurfaceKind::Content, argument) => format!("content:{}", argument.unwrap_or("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn writes_messages_as_tagged_json() {
        let render = Outgoing::Render {
            view: view_id(SurfaceKind::Document, Some("a")),
            surface: SurfaceKind::Document,
            content: None,
            argument: Some("a".into()),
            session_dir: None,
        };
        assert_eq!(
            serde_json::to_value(render).unwrap(),
            json!({"type": "render", "view": "document:a", "surface": "document", "argument": "a"})
        );
    }

    #[test]
    fn content_source_has_a_typed_payload_and_errors_target_its_stable_view() {
        let message = Outgoing::Render {
            view: view_id(SurfaceKind::Content, Some("entry-1-block-2")),
            surface: SurfaceKind::Content,
            argument: Some("entry-1-block-2".into()),
            content: Some(Content {
                language: "future-figure".into(),
                source: "original source\n".into(),
            }),
            session_dir: Some("/fixture/chat-one".into()),
        };
        let value = serde_json::to_value(message).unwrap();
        assert_eq!(
            value["content"],
            json!({"language":"future-figure","source":"original source\n"})
        );
        assert_eq!(value["view"], "content:entry-1-block-2");
        let error = parse(r#"{"type":"content_error","view":"content:entry-1-block-2","session_dir":"/fixture/chat-one","error":"unsupported source","sequence":4}"#).unwrap();
        assert!(matches!(error, Incoming::ContentError { sequence: 4, .. }));
    }

    #[test]
    fn reads_views_and_rejects_invalid_trees() {
        let view = parse(r#"{"type": "view", "view": "panel", "root": {"type": "divider"}}"#);
        assert!(matches!(view, Ok(Incoming::View { .. })));
        assert!(parse(r#"{"type": "view", "view": "panel", "root": {"type": "nope"}}"#).is_ok());
        assert!(parse(r#"{"type": "view", "view": "panel", "root": {"type": "text"}}"#).is_err());
        assert!(parse(r#"{"type": "log", "text": "x"}"#).is_err());
        // Value types belong to the advertised setting, not the native router.
        assert!(matches!(
            parse(
                r#"{"type":"set_session","id":"a","session_dir":"/s","setting":{"id":"org.example/flag","value":true}}"#
            ),
            Ok(Incoming::SetSession { .. })
        ));
        assert!(parse(r#"{"type":"set_session","id":"a","session_dir":"/s","setting":{"type":"mode","value":"auto"}}"#).is_err());
    }

    #[test]
    fn invalid_content_trees_keep_their_view_session_and_action_sequence() {
        for source in [
            "<svg",
            "<svg><image href=\"/tmp/private.png\"/></svg>",
            "<svg><foreignObject/></svg>",
        ] {
            // The SVG can be nested in any renderer's public widget tree.
            let root = json!({"type":"stack", "children":[{
                "type":"svg", "source":source, "width":640, "height":360
            }]});
            let mut envelope = json!({
                "type":"view", "view":"content:broken-block",
                "session_dir":"/fixture/chat-one", "sequence":7, "root":root
            });
            let failure = parse(&envelope.to_string()).unwrap();
            assert!(matches!(failure, Incoming::ContentError {
                view, session_dir: Some(session), sequence: 7, error,
            } if view == "content:broken-block"
                && session == std::path::Path::new("/fixture/chat-one")
                && !error.is_empty()));

            // A subsequent healthy block remains a normal view; it cannot
            // clear the failure addressed to broken-block.
            envelope["view"] = json!("content:healthy-block");
            envelope["root"] = json!({"type":"text", "text":"healthy"});
            assert!(matches!(
                parse(&envelope.to_string()).unwrap(),
                Incoming::View { .. }
            ));

            envelope["view"] = json!("panel");
            envelope["root"] = root;
            assert!(parse(&envelope.to_string()).is_err());
        }
    }

    #[test]
    fn validates_independent_badge_presentations() {
        let badge = r#"{"type":"badge","view":"badge","session_dir":"/a","title":"Left 8%","lines":["Weekly"],"face":{"type":"ring","percent":92,"tone":"danger"}}"#;
        assert!(matches!(parse(badge).unwrap(), Incoming::Badge { .. }));
        assert!(parse(&badge.replace("\"percent\":92", "\"percent\":101")).is_err());
        assert!(parse(&badge.replace("\"type\":\"ring\"", "\"type\":\"unknown\"")).is_err());
    }

    #[test]
    fn shipped_processes_conform_to_the_public_ui_protocol() {
        let root = std::env::var_os("PROTEUS_TEST_EXTENSIONS_SOURCE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../Proteus-desktop-extensions")
            });
        assert!(
            root.join("package.py").is_file(),
            "run python3 scripts/test.py to prepare the separately versioned extensions"
        );
        let (packages, errors) = crate::declarative::manifest::discover(&root.join("packages"));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(packages.len(), 11);
        let output = std::process::Command::new("python3")
            .arg(root.join("tests/wire.py"))
            .arg("--wire")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let wire = String::from_utf8(output.stdout).unwrap();
        let mut count = 0;
        for line in wire.lines() {
            parse(line).unwrap_or_else(|error| panic!("invalid extension output: {error}: {line}"));
            count += 1;
        }
        assert!(count >= 30, "not all declared surfaces were exercised");
    }
}
