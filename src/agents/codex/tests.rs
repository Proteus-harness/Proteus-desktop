use super::*;

const PEER: &str = r#"
import json,sys
counter=0
model_requests=0
resumes={}
loaded=set()
for line in sys.stdin:
    msg=json.loads(line)
    method=msg.get('method')
    if 'id' not in msg: continue
    p=msg.get('params',{})
    if method=='initialize': result={}
    elif method=='thread/list':
        assert p['limit']==50 and p['useStateDbOnly'] and p['modelProviders']==[]
        assert p['sourceKinds']==['cli','vscode','exec','appServer','subAgentThreadSpawn']
        result={'data':[{'id':'opaque:stored','cwd':p['cwd'],'name':'Stored','updatedAt':12,'parentThreadId':'opaque:parent'}],'nextCursor':'next:page'}
    elif method=='model/list':
        model_requests+=1
        print(json.dumps({'id':'server:opaque:request','method':'attestation/generate','params':{'nonce':'synthetic'}}),flush=True)
        rejection=json.loads(next(sys.stdin))
        assert rejection['id']=='server:opaque:request' and rejection['error']['code']==-32601
        result={'data':[{'id':'catalog:id','model':'execution-model','displayName':'Model','description':'','isDefault':True,'defaultReasoningEffort':'high','supportedReasoningEfforts':[{'reasoningEffort':'high'}]}],'nextCursor':None}
    elif method=='thread/start':
        assert p['historyMode']=='paginated'
        counter+=1
        thread='opaque:new:'+str(counter)
        loaded.add(thread)
        print(json.dumps({'method':'turn/started','params':{'threadId':thread,'turn':{'id':'early:turn'}}}),flush=True)
        print(json.dumps({'method':'turn/completed','params':{'threadId':thread,'turn':{'id':'early:turn','status':'completed'}}}),flush=True)
        result={'thread':{'id':thread,'cwd':p['cwd'],'preview':'','turns':[]},'model':'execution-model'}
    elif method=='thread/resume':
        resumes[p['threadId']]=resumes.get(p['threadId'],0)+1
        loaded.add(p['threadId'])
        assert p['excludeTurns'] is True and 'includeTurns' not in p
        result={'thread':{'id':p['threadId'],'cwd':'/synthetic','preview':'stored','turns':[],'status':{'type':'active' if p['threadId'] in ('opaque:active','opaque:delete-fails-active') else 'idle'}},'model':'execution-model'}
    elif method=='thread/turns/list':
        assert p['itemsView']=='notLoaded' and p['limit']==1
        result={'data':[{'id':'active:turn','status':'inProgress','items':[]}],'nextCursor':None}
    elif method=='thread/items/list':
        assert p['limit']==50 and p['sortDirection']=='desc'
        result={'data':[{'turnId':'old:turn','item':{'id':'assistant:2','type':'agentMessage','text':'answer'}},{'turnId':'old:turn','item':{'id':'user:1','type':'userMessage','content':[{'type':'text','text':'question'}]}}],'nextCursor':'older:items'}
    elif method=='turn/start':
        assert p['input'][0]['text']=='synthetic hello'
        result={'turn':{'id':'new:turn'}}
    elif method=='test/modelCount': result=model_requests
    elif method=='test/resumes': result=resumes.get(p['threadId'],0)
    elif method=='test/deletedBatch':
        for thread in p['ids']:
            print(json.dumps({'method':'thread/deleted','params':{'threadId':thread}}),flush=True)
        result={}
    elif method=='thread/delete':
        assert p['threadId'] not in loaded, 'unsubscribe must precede delete'
        if p['threadId'] in ('opaque:delete-fails','opaque:delete-fails-active'):
            print(json.dumps({'id':msg['id'],'error':{'code':-32000,'message':'synthetic delete failure'}}),flush=True)
            continue
        result={}
        deleted=[p['threadId']]
        if p['threadId']=='opaque:cascade': deleted+=['opaque:child','opaque:unopened-grandchild']
        for thread in deleted:
            print(json.dumps({'method':'thread/deleted','params':{'threadId':thread}}),flush=True)
    elif method=='thread/unsubscribe':
        loaded.discard(p['threadId'])
        result={}
    elif method=='turn/interrupt': result={}
    else:
        print(json.dumps({'id':msg['id'],'error':{'code':-32601,'message':'unknown synthetic method'}}),flush=True)
        continue
    print(json.dumps({'id':msg['id'],'result':result}),flush=True)
"#;

fn peer() -> CodexService {
    peer_with(PEER)
}

fn peer_with(script: &str) -> CodexService {
    let mut launch = Launch::codex(PathBuf::from("python3"), std::env::current_dir().unwrap());
    launch.args = vec!["-u".into(), "-c".into(), script.into()];
    launch.limits.request_timeout = Duration::from_secs(3);
    CodexService::launch(launch).unwrap()
}

/// Hold RPC replies until explicit barriers, without sleeping or using real threads.
const SUBSCRIPTION_PEER: &str = r#"
import json,sys
pending={}
barriers={}
calls=[]
subscribed=False
def emit(rid,result): print(json.dumps({'id':rid,'result':result}),flush=True)
def mark(method):
    calls.append(method)
    for rid in barriers.pop(method,[]): emit(rid,{})
for line in sys.stdin:
    msg=json.loads(line)
    if 'id' not in msg: continue
    method=msg['method']; p=msg.get('params',{}); rid=msg['id']
    if method=='thread/resume':
        subscribed=True
        result={'thread':{'id':p['threadId'],'cwd':'/synthetic','turns':[],'status':{'type':'idle'}}}
    elif method=='thread/items/list': result={'data':[],'nextCursor':None}
    elif method=='turn/start':
        assert subscribed, 'turn/start must not follow unsubscribe'
        pending[method]=rid; mark(method); continue
    elif method=='thread/unsubscribe':
        assert 'turn/start' not in pending, 'unsubscribe raced an unacknowledged turn/start'
        subscribed=False; mark(method); result={}
    elif method=='thread/delete':
        assert not subscribed
        pending[method]=rid; mark(method); continue
    elif method=='test/barrier':
        if p['method'] in pending: result={}
        else: barriers.setdefault(p['method'],[]).append(rid); continue
    elif method=='test/release':
        target=p['method']
        emit(pending.pop(target),{'turn':{'id':'guarded-turn'}} if target=='turn/start' else {})
        if target=='thread/delete': print(json.dumps({'method':'thread/deleted','params':{'threadId':'guarded'}}),flush=True)
        result={}
    elif method=='test/calls': result=calls
    else: result={}
    emit(rid,result)
"#;

#[test]
fn delete_in_flight_rejects_send_without_submitting_a_turn() {
    let service = peer_with(SUBSCRIPTION_PEER);
    let opened = service.resume("guarded").unwrap();
    std::thread::scope(|scope| {
        let deleting = scope.spawn(|| service.delete_thread("guarded"));
        service
            .client
            .request("test/barrier", json!({"method":"thread/delete"}))
            .unwrap();
        assert_eq!(
            opened.events.recv_blocking().unwrap(),
            Event::SubscriptionSuspended
        );
        assert!(!opened.handle.is_connected());
        assert!(opened.handle.send("synthetic hello", None, None).is_err());
        assert_eq!(
            service.client.request("test/calls", json!({})).unwrap(),
            json!(["thread/unsubscribe", "thread/delete"])
        );
        service
            .client
            .request("test/release", json!({"method":"thread/delete"}))
            .unwrap();
        deleting.join().unwrap().unwrap();
    });
}

#[test]
fn an_earlier_send_is_acknowledged_before_delete_unsubscribes() {
    let service = peer_with(SUBSCRIPTION_PEER);
    let opened = service.resume("guarded").unwrap();
    std::thread::scope(|scope| {
        let sending = scope.spawn(|| opened.handle.send("synthetic hello", None, None));
        service
            .client
            .request("test/barrier", json!({"method":"turn/start"}))
            .unwrap();
        let subscription = service.subscriptions.for_thread("guarded");
        assert!(
            subscription.try_lock().is_err(),
            "turn/start must hold the subscription guard through its reply"
        );
        let deleting = scope.spawn(|| service.delete_thread("guarded"));
        service
            .client
            .request("test/release", json!({"method":"turn/start"}))
            .unwrap();
        assert_eq!(sending.join().unwrap().unwrap(), "guarded-turn");
        service
            .client
            .request("test/barrier", json!({"method":"thread/delete"}))
            .unwrap();
        assert_eq!(
            service.client.request("test/calls", json!({})).unwrap(),
            json!(["turn/start", "thread/unsubscribe", "thread/delete"])
        );
        service
            .client
            .request("test/release", json!({"method":"thread/delete"}))
            .unwrap();
        deleting.join().unwrap().unwrap();
    });
}

#[test]
fn concurrent_chat_opens_share_one_model_catalog_for_the_service() {
    let service = peer();
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..8)
            .map(|_| {
                let service = service.clone();
                scope.spawn(move || service.models().unwrap())
            })
            .collect();
        for job in jobs {
            assert_eq!(job.join().unwrap()[0].model, "execution-model");
        }
    });
    assert_eq!(
        service
            .client
            .request("test/modelCount", json!({}))
            .unwrap(),
        1
    );
}

#[test]
fn failed_catalog_is_retried_and_a_successful_empty_catalog_is_cached() {
    let service = peer_with(
        r#"
import json,sys
count=0
for line in sys.stdin:
    msg=json.loads(line)
    if 'id' not in msg: continue
    if msg['method']=='model/list':
        count+=1
        if count==1:
            print(json.dumps({'id':msg['id'],'error':{'code':-32000,'message':'retry'}}),flush=True)
            continue
        result={'data':[],'nextCursor':None}
    elif msg['method']=='test/modelCount': result=count
    else: result={}
    print(json.dumps({'id':msg['id'],'result':result}),flush=True)
"#,
    );
    assert!(service.models().is_err());
    assert!(service.models().unwrap().is_empty());
    assert!(service.clone().models().unwrap().is_empty());
    assert_eq!(
        service
            .client
            .request("test/modelCount", json!({}))
            .unwrap(),
        2
    );
}

#[test]
fn a_slow_resume_does_not_serialize_opening_other_chats() {
    let service = peer_with(
        r#"
import json,sys
waiting=None
barrier=None
def emit(rid,result): print(json.dumps({'id':rid,'result':result}),flush=True)
def resumed(thread): return {'thread':{'id':thread,'cwd':'/synthetic','turns':[],'status':{'type':'idle'}}}
for line in sys.stdin:
    msg=json.loads(line)
    if 'id' not in msg: continue
    method=msg['method']; p=msg.get('params',{}); rid=msg['id']
    if method=='thread/resume' and p['threadId']=='slow':
        waiting=rid
        if barrier is not None: emit(barrier,{})
        continue
    if method=='test/waiting':
        if waiting is None: barrier=rid
        else: emit(rid,{})
        continue
    if method=='thread/resume':
        emit(rid,resumed(p['threadId']))
        if waiting is not None:
            emit(waiting,resumed('slow')); waiting=None
        continue
    result={'data':[],'nextCursor':None} if method=='thread/items/list' else {}
    emit(rid,result)
"#,
    );
    std::thread::scope(|scope| {
        let slow = scope.spawn(|| service.resume("slow"));
        service.client.request("test/waiting", json!({})).unwrap();
        let fast = service.resume("fast").unwrap();
        assert_eq!(fast.metadata.id, "fast");
        assert_eq!(slow.join().unwrap().unwrap().metadata.id, "slow");
    });
}

#[test]
fn service_routes_early_events_lists_opaque_metadata_and_caps_history() {
    let service = peer();
    let page = service.list(Path::new("/synthetic"), None).unwrap();
    assert_eq!(page.data[0].id, "opaque:stored");
    assert_eq!(
        page.data[0].parent_thread_id.as_deref(),
        Some("opaque:parent")
    );
    assert_eq!(page.next_cursor.as_deref(), Some("next:page"));
    let models = service.models().unwrap();
    assert!(
        service.is_connected(),
        "unsupported global request poisoned the service"
    );
    assert_eq!(models[0].model, "execution-model");
    assert_eq!(models[0].efforts, vec!["high"]);
    let opened = service.new_thread(Path::new("/synthetic")).unwrap();
    assert_eq!(opened.handle.thread_id(), "opaque:new:1");
    assert!(opened.history.data.is_empty());
    assert_eq!(
        opened.events.recv_blocking().unwrap(),
        Event::TurnStarted {
            id: "early:turn".into()
        }
    );
    assert_eq!(
        opened.events.recv_blocking().unwrap(),
        Event::TurnCompleted {
            id: "early:turn".into(),
            status: "completed".into(),
            error: None
        }
    );
    let resumed = service.resume("opaque:stored").unwrap();
    assert_eq!(resumed.history.data[0].id, "user:1");
    assert_eq!(resumed.history.data[1].id, "assistant:2");
    assert_eq!(resumed.history.next_cursor.as_deref(), Some("older:items"));
    assert_eq!(
        resumed
            .handle
            .send("synthetic hello", Some("execution-model"), Some("high"))
            .unwrap(),
        "new:turn"
    );
    assert!(resumed.handle.send("synthetic hello", None, None).is_err());
    resumed.handle.interrupt("new:turn").unwrap();
    service.delete_thread("opaque:stored").unwrap();
}

#[test]
fn cloned_handle_does_not_unregister_original_route() {
    let service = peer();
    let opened = service.resume("opaque:stored").unwrap();
    let clone = opened.handle.clone();
    drop(clone);
    assert!(
        service
            .router
            .lock()
            .unwrap()
            .register("opaque:stored")
            .is_err()
    );
    assert_eq!(opened.handle.history(None).unwrap().data.len(), 2);
    drop(opened);
    let reopened = service.resume("opaque:stored").unwrap();
    assert_eq!(reopened.metadata.id, "opaque:stored");
}

#[test]
fn active_resume_loads_only_current_turn_metadata_and_can_interrupt() {
    let service = peer();
    let opened = service.resume("opaque:active").unwrap();
    assert!(opened.metadata.active);
    assert_eq!(opened.handle.current_turn().as_deref(), Some("active:turn"));
    assert!(opened.handle.send("synthetic hello", None, None).is_err());
    opened.handle.interrupt("active:turn").unwrap();
}

#[test]
fn definite_delete_failure_restores_same_handle_and_bounded_tail() {
    let service = peer();
    let failed = service.resume("opaque:delete-fails").unwrap();
    let other = service.resume("opaque:other").unwrap();
    assert!(service.delete_thread("opaque:delete-fails").is_err());
    assert!(!failed.events.is_closed());
    assert!(failed.handle.is_connected());
    assert_eq!(
        failed.events.recv_blocking().unwrap(),
        Event::SubscriptionSuspended
    );
    assert_eq!(failed.events.recv_blocking().unwrap(), Event::ResyncStarted);
    let Event::Resynced {
        current_turn,
        history,
    } = failed.events.recv_blocking().unwrap()
    else {
        panic!("missing recovery snapshot");
    };
    assert!(current_turn.is_none());
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].id, "user:1");
    assert!(!other.events.is_closed());
    assert!(other.handle.history(None).is_ok());
    assert!(service.is_connected());
    assert!(
        service.resume("opaque:delete-fails").is_err(),
        "recovery must not register a duplicate route"
    );
    assert_eq!(
        failed.handle.send("synthetic hello", None, None).unwrap(),
        "new:turn"
    );
}

#[test]
fn definite_delete_failure_restores_active_turn_without_loading_full_turns() {
    let service = peer();
    let opened = service.resume("opaque:delete-fails-active").unwrap();
    assert!(service.delete_thread("opaque:delete-fails-active").is_err());
    assert_eq!(
        opened.events.recv_blocking().unwrap(),
        Event::SubscriptionSuspended
    );
    assert_eq!(opened.events.recv_blocking().unwrap(), Event::ResyncStarted);
    let Event::Resynced { current_turn, .. } = opened.events.recv_blocking().unwrap() else {
        panic!("missing recovery snapshot");
    };
    assert_eq!(current_turn.as_deref(), Some("active:turn"));
    assert_eq!(opened.handle.current_turn().as_deref(), Some("active:turn"));
    opened.handle.interrupt("active:turn").unwrap();
}

#[test]
fn ambiguous_delete_timeout_invalidates_handle_without_automatic_resume() {
    let script = PEER.replace("result={}\n        deleted=[p['threadId']]", "if p['threadId']=='opaque:delete-timeout': continue\n        result={}\n        deleted=[p['threadId']]");
    let mut launch = Launch::codex(PathBuf::from("python3"), std::env::current_dir().unwrap());
    launch.args = vec!["-u".into(), "-c".into(), script.into()];
    launch.limits.request_timeout = Duration::from_millis(250);
    let service = CodexService::launch(launch).unwrap();
    let opened = service.resume("opaque:delete-timeout").unwrap();
    assert_eq!(
        service.delete_thread("opaque:delete-timeout"),
        Err(Error::TimedOut)
    );
    assert!(!opened.handle.is_connected());
    assert!(opened.events.is_closed());
    assert!(opened.handle.send("synthetic hello", None, None).is_err());
    assert_eq!(
        service
            .client
            .request("test/resumes", json!({"threadId":"opaque:delete-timeout"}))
            .unwrap(),
        1
    );
    let reopened = service.resume("opaque:delete-timeout").unwrap();
    assert!(
        !opened.handle.is_connected(),
        "an old generation must not borrow its replacement's connection"
    );
    drop(opened);
    assert!(reopened.handle.history(None).is_ok());
}

#[test]
fn authoritative_catalog_deletion_closes_descendant_routes_and_has_one_owner() {
    let service = peer();
    let catalog = service.take_catalog_events().unwrap();
    assert!(service.clone().take_catalog_events().is_none());
    let root = service.resume("opaque:cascade").unwrap();
    let child = service.resume("opaque:child").unwrap();
    let other = service.resume("opaque:other").unwrap();
    service.delete_thread("opaque:cascade").unwrap();
    let ids: Vec<_> = (0..3)
        .map(|_| match catalog.recv_blocking().unwrap() {
            CatalogEvent::ThreadDeleted { id } => id,
        })
        .collect();
    assert_eq!(
        ids,
        [
            "opaque:cascade",
            "opaque:child",
            "opaque:unopened-grandchild"
        ]
    );
    assert!(!root.handle.is_connected());
    assert!(!child.handle.is_connected());
    assert!(root.events.is_closed() && child.events.is_closed());
    assert!(other.handle.is_connected());
    assert!(other.handle.history(None).is_ok());
}

#[test]
fn catalog_backpressure_keeps_every_id_without_holding_the_route_mutex() {
    let service = peer();
    let catalog = service.take_catalog_events().unwrap();
    let wait = |condition: &dyn Fn() -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "synthetic routing barrier timed out"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    for batch in 0..4 {
        let ids: Vec<_> = (batch * 64..(batch + 1) * 64)
            .map(|n| format!("deleted:{n}"))
            .collect();
        service
            .client
            .request("test/deletedBatch", json!({"ids":ids}))
            .unwrap();
        wait(&|| catalog.len() == (batch + 1) * 64);
    }
    let blocked = service.resume("blocked-route").unwrap();
    service
        .client
        .request("test/deletedBatch", json!({"ids":["blocked-route"]}))
        .unwrap();
    wait(&|| !blocked.handle.is_connected());
    assert_eq!(catalog.len(), 256);
    // The router thread is now blocked on metadata delivery, but another
    // route's registration and direct RPCs must remain usable.
    let other = service.resume("other").unwrap();
    assert!(other.handle.is_connected());
    for n in 0..256 {
        assert_eq!(
            catalog.recv_blocking().unwrap(),
            CatalogEvent::ThreadDeleted {
                id: format!("deleted:{n}")
            }
        );
    }
    assert_eq!(
        catalog.recv_blocking().unwrap(),
        CatalogEvent::ThreadDeleted {
            id: "blocked-route".into()
        }
    );
}

/// Explicit local protocol smoke: no turns, prompts, account reads or auth files.
#[test]
#[ignore = "starts the installed Codex app-server; run explicitly"]
fn installed_app_server_smoke_without_turns() {
    let program = std::env::var_os("PROTEUS_CODEX_SMOKE_PROGRAM")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"));
    let cwd = std::env::current_dir()
        .unwrap()
        .join("build/verification/codex-driver-smoke");
    std::fs::create_dir_all(&cwd).unwrap();
    let service = CodexService::spawn(program, cwd.clone()).unwrap();
    let models = service.models().unwrap();
    assert!(!models.is_empty());
    let opened = service.new_thread(&cwd).unwrap();
    let id = opened.metadata.id.clone();
    assert!(opened.history.data.is_empty());
    assert!(opened.handle.current_turn().is_none());
    service.delete_thread(&id).unwrap();
    drop(opened);
    let closed = service.new_thread(&cwd).unwrap();
    let closed_id = closed.metadata.id.clone();
    service
        .client
        .request("thread/unsubscribe", json!({"threadId":closed_id}))
        .unwrap();
    drop(closed);
    // Deleting a closed sidebar entry must also accept an already-unloaded
    // server subscription, not regress while fixing deletion of an open tab.
    service.delete_thread(&closed_id).unwrap();
    assert!(service.is_connected());
}
