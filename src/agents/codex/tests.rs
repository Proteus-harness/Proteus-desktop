use super::*;

const PEER: &str = r#"
import json,sys
counter=0
model_requests=0
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
        loaded.add(p['threadId'])
        assert p['excludeTurns'] is True and 'includeTurns' not in p
        result={'thread':{'id':p['threadId'],'cwd':'/synthetic','preview':'stored','turns':[],'status':{'type':'active' if p['threadId']=='opaque:active' else 'idle'}},'model':'execution-model'}
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
    elif method=='thread/delete':
        assert p['threadId'] not in loaded, 'unsubscribe must precede delete'
        if p['threadId']=='opaque:delete-fails':
            print(json.dumps({'id':msg['id'],'error':{'code':-32000,'message':'synthetic delete failure'}}),flush=True)
            continue
        result={}
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
fn delete_failure_invalidates_only_its_subscription_without_replaying_resume() {
    let service = peer();
    let failed = service.resume("opaque:delete-fails").unwrap();
    let other = service.resume("opaque:other").unwrap();
    assert!(service.delete_thread("opaque:delete-fails").is_err());
    assert!(failed.events.is_closed());
    assert!(
        failed
            .events
            .disconnect_reason()
            .unwrap()
            .contains("synthetic delete failure")
    );
    assert!(!other.events.is_closed());
    assert!(other.handle.history(None).is_ok());
    assert!(service.is_connected());
    // Only a subsequent explicit open registers a replacement route. Even the
    // old handle dropping later must not unsubscribe that replacement.
    let reopened = service.resume("opaque:delete-fails").unwrap();
    drop(failed);
    assert!(reopened.handle.history(None).is_ok());
    assert!(!reopened.events.is_closed());
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
