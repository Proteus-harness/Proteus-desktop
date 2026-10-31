use super::*;
use std::future::Future;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll, Wake, Waker};

#[test]
fn ready_batches_leave_overflow_in_fifo_order_including_barriers() {
    let (send, receive) = async_channel::unbounded();
    let mut expected = Vec::new();
    for index in 0..MAX_READY_EVENTS * 2 + 3 {
        let item = match index {
            value if value == MAX_READY_EVENTS - 1 => PumpItem::Disconnected("offline".into()),
            value if value == MAX_READY_EVENTS => PumpItem::Connected,
            value if value == MAX_READY_EVENTS + 1 => {
                PumpItem::Event(Box::new(snapshot("second", 0, 10)))
            }
            value if value == MAX_READY_EVENTS * 2 => {
                PumpItem::Event(Box::new(AppServerEvent::Shutdown))
            }
            _ => PumpItem::Event(text(index as u64 + 11)),
        };
        expected.push(wire(&item));
        assert!(send.try_send(item).is_ok());
    }
    drop(send);
    let first = receive.try_recv().unwrap();
    let first = ready_batch(first, &receive);
    assert_eq!(first.len(), MAX_READY_EVENTS);
    assert_eq!(receive.len(), MAX_READY_EVENTS + 3);
    let mut actual = Vec::new();
    let mut lengths = Vec::new();
    let mut current = Some(first);
    while let Some(batch) = current.take() {
        lengths.push(batch.len());
        for item in batches(batch) {
            match item {
                BatchItem::Text(run) => actual.extend(
                    run.into_events()
                        .iter()
                        .map(|event| serde_json::to_value(event).unwrap()),
                ),
                BatchItem::Barrier(item) => actual.push(wire(&item)),
            }
        }
        current = receive
            .try_recv()
            .ok()
            .map(|first| ready_batch(first, &receive));
    }
    assert_eq!(lengths, [MAX_READY_EVENTS, MAX_READY_EVENTS, 3]);
    assert_eq!(actual, expected);
}

#[test]
fn partial_batch_returns_without_waiting_for_the_budget() {
    let (send, receive) = async_channel::unbounded();
    let batch = ready_batch(PumpItem::Connected, &receive);
    assert_eq!(batch.len(), 1);
    assert!(!receive.is_closed());
    assert!(
        send.try_send(PumpItem::Disconnected("later".into()))
            .is_ok()
    );
    assert_eq!(
        wire(&receive.try_recv().unwrap()),
        json!({"connection":"disconnected", "error":"later"})
    );
}

#[test]
fn text_reducer_matches_unbatched_oracle_across_ready_limits() {
    let (send, receive) = async_channel::unbounded();
    let mut source = String::new();
    for index in 0..MAX_READY_EVENTS * 2 + 3 {
        let offset = if index == MAX_READY_EVENTS - 1 {
            8
        } else {
            source.len()
        };
        source.truncate(offset);
        source.push_str("🦀");
        assert!(
            send.try_send(PumpItem::Event(event(
                Event::AssistantTextDelta {
                    offset,
                    message_id: id(100),
                    phase: None,
                    text: "🦀".into(),
                },
                index as u64 + 11
            )))
            .is_ok()
        );
    }
    let (mut actual, _) = ready();
    let (mut expected, _) = ready();
    while let Ok(first) = receive.try_recv() {
        for item in batches(ready_batch(first, &receive)) {
            let BatchItem::Text(run) = item else {
                panic!("text-only fixture")
            };
            let events = run.clone().into_events();
            let (changes, _) = oracle(&mut expected, &events);
            assert_eq!(
                actual
                    .apply_text_run(run)
                    .event_changes()
                    .collect::<Vec<_>>(),
                changes
            );
            assert_eq!(observed_rows(&actual), observed_rows(&expected));
        }
    }
    assert_eq!(&*actual.transcript.entries()[0].text, source);
}

struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn every_batch_yield_returns_pending_once_and_requeues_itself() {
    let count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(count.clone());
    let mut cx = Context::from_waker(&waker);
    for expected in 1..=3 {
        let mut yielding = Box::pin(yield_after_batch());
        assert_eq!(yielding.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(count.0.load(Ordering::Relaxed), expected);
        assert_eq!(yielding.as_mut().poll(&mut cx), Poll::Ready(()));
        assert_eq!(count.0.load(Ordering::Relaxed), expected);
    }
}
