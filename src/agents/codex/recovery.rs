//! Restore only definitively rejected deletions, using the existing route.
use super::{CodexService, Error, ThreadHandle, array, metadata};
use serde_json::{Value, json};

impl CodexService {
    pub(super) fn recover_subscription(&self, id: &str) -> bool {
        let Some(generation) = self.router.lock().unwrap().begin_resync(id) else {
            // A closed tab must not acquire a new server subscription.
            return false;
        };
        let snapshot = (|| {
            let response = self.client.request(
                "thread/resume",
                json!({"threadId": id, "excludeTurns": true}),
            )?;
            let metadata = metadata(&response)?;
            if metadata.id != id {
                return Err(Error::Protocol("Codex resumed a different thread".into()));
            }
            let current_turn = if metadata.active {
                let turns = self.client.request(
                    "thread/turns/list",
                    json!({"threadId": id, "limit": 1, "sortDirection": "desc", "itemsView": "notLoaded"}),
                )?;
                array(&turns, "data")?
                    .first()
                    .filter(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))
                    .and_then(|turn| turn.get("id").and_then(Value::as_str))
                    .map(str::to_owned)
            } else {
                None
            };
            let history = ThreadHandle::load_history(
                &self.client,
                id,
                response.get("itemsBackwardsCursor").and_then(Value::as_str),
            )?;
            Ok::<_, Error>((current_turn, history.data))
        })();
        match snapshot {
            Ok((current_turn, history)) => {
                self.router
                    .lock()
                    .unwrap()
                    .finish_resync(id, generation, current_turn, history)
            }
            Err(_) => false,
        }
    }
}
