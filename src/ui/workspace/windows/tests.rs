use super::*;

#[::core::prelude::v1::test]
fn actual_backend_exit_only_releases_the_matching_shared_connection() {
    let cx = ::gpui::TestAppContext::single();
    let windows = WindowGroup::default();
    let old = Api::new("http://127.0.0.1:1", "synthetic-old");
    let current = Api::new("http://127.0.0.1:1", "synthetic-current");
    windows.set_core(current.clone(), None);

    cx.update(|cx| windows.server_stopped(old.connection_id(), "late old process exit", cx));
    assert!(windows.has_core_connection(current.connection_id()));
    cx.update(|cx| windows.server_stopped(current.connection_id(), "actual process exited", cx));
    assert!(!windows.has_core_connection(current.connection_id()));
    assert!(windows.core_slot().lock().unwrap().is_none());
}
