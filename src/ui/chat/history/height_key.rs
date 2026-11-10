//! Persisted geometry follows the saved conversation, not a runtime incarnation.
use crate::ui::chat::{
    Session,
    scroller::{HeightCacheKey, height_hash},
};
use proteus_contracts::app_protocol::AppTranscriptCursor;

pub(super) fn key(session: &Session, cursor: &AppTranscriptCursor) -> Option<HeightCacheKey> {
    let id = session.durable_id()?;
    Some(HeightCacheKey {
        identity: height_hash(format!("{}:{id}", session.key.tab_id())),
        revision: height_hash(&cursor.revision),
        layout: [0; 32],
        // The cursor owns the archived prefix; its last row also depends on
        // the mutable successor's group/footer. Runtime roots may be absent
        // on idle reopen and are recreated by Core when execution resumes.
        prefix: cursor.before.saturating_sub(1),
        base: cursor.base.as_ref().map(|base| {
            (
                height_hash(&base.revision),
                base.common_prefix.saturating_sub(1),
            )
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::chat::history::History;
    use proteus_contracts::app_protocol::AppSessionSnapshot;
    use uuid::Uuid;

    #[test]
    fn idle_reopen_and_new_runtime_roots_share_the_saved_session_geometry() {
        let mut session = Session::new("/synthetic/saved-chat".into());
        let mut history = History::default();
        let cursor = AppTranscriptCursor {
            revision: "canonical".into(),
            before: 550,
            base: None,
        };
        assert!(
            key(&session, &cursor).is_none(),
            "unidentified chats cannot share disk geometry"
        );
        let snapshot = |id, stream: &str, root| AppSessionSnapshot {
            session_id: id,
            stream_id: stream.into(),
            seq: 0,
            root_thread_id: root,
            transcript: vec![],
            history: Some(cursor.clone()),
            execution: Default::default(),
        };
        let id = Uuid::from_u128(1);
        // Actual Core idle restore: the progress projector has no root yet.
        session.apply_snapshot(snapshot(id, "first-process", None), &mut history);
        let saved = key(&session, history.cursor.as_ref().unwrap())
            .expect("idle restored chat must connect its persisted geometry");
        session.apply_snapshot(snapshot(id, "second-process", None), &mut history);
        assert_eq!(
            key(&session, history.cursor.as_ref().unwrap()),
            Some(saved.clone())
        );
        session.apply_snapshot(
            snapshot(id, "second-process", Some(Uuid::from_u128(10))),
            &mut history,
        );
        assert_eq!(
            key(&session, history.cursor.as_ref().unwrap()),
            Some(saved.clone())
        );
        session.apply_snapshot(
            snapshot(id, "third-process", Some(Uuid::from_u128(20))),
            &mut history,
        );
        assert_eq!(
            key(&session, history.cursor.as_ref().unwrap()),
            Some(saved.clone())
        );
        session.apply_snapshot(
            snapshot(Uuid::from_u128(2), "third-process", None),
            &mut history,
        );
        assert_ne!(
            key(&session, history.cursor.as_ref().unwrap())
                .unwrap()
                .identity,
            saved.identity
        );
    }
}
