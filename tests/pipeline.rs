use serde_json::{Value, json};
use starknet_launch_watch::{
    config::{Registry, Settings},
    decode::{Receipt, decode_receipt},
    encoding::selector,
    engine::Engine,
    enrich::{self, TokenInfo},
    rpc::Rpc,
    store::{Alert, Store},
};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn registry() -> Registry {
    Registry::load("config/mainnet.json").unwrap()
}
fn receipts() -> Vec<Receipt> {
    serde_json::from_str(include_str!("fixtures/mainnet-receipts.json")).unwrap()
}
fn first_launch() -> starknet_launch_watch::decode::Launch {
    decode_receipt(&registry(), &receipts()[0])
        .unwrap()
        .remove(0)
}

#[test]
fn real_mainnet_receipts_distinguish_creation_launch_and_pool() {
    let launches: Vec<_> = receipts()
        .iter()
        .flat_map(|r| decode_receipt(&registry(), r).unwrap())
        .collect();
    assert_eq!(launches.len(), 6);
    assert_eq!(
        launches
            .iter()
            .filter(|l| l.kind == "MemecoinCreated")
            .count(),
        2
    );
    assert_eq!(
        launches
            .iter()
            .filter(|l| l.kind == "MemecoinLaunched")
            .count(),
        2
    );
    assert_eq!(
        launches
            .iter()
            .filter(|l| l.kind == "PoolInitialized")
            .count(),
        2
    );
    assert!(
        launches
            .iter()
            .filter(|l| l.kind == "PoolInitialized")
            .all(|l| l.pool.is_some())
    );
}
#[test]
fn identical_events_in_one_transaction_keep_distinct_receipt_indices() {
    let mut r = receipts().remove(0);
    let e = r
        .events
        .iter()
        .find(|e| e.keys.first() == Some(&selector("MemecoinCreated")))
        .unwrap()
        .clone();
    r.events.push(e);
    let l = decode_receipt(&registry(), &r).unwrap();
    assert_eq!(l.len(), 2);
    assert_ne!(l[0].id, l[1].id);
}
#[test]
fn malformed_tracked_event_is_not_silently_skipped() {
    let mut r = receipts().remove(0);
    r.events
        .iter_mut()
        .find(|e| e.keys.first() == Some(&selector("MemecoinCreated")))
        .unwrap()
        .data
        .pop();
    assert!(decode_receipt(&registry(), &r).is_err());
}
#[test]
fn reverted_and_untracked_events_do_not_create_alerts() {
    let mut r = receipts().remove(0);
    r.execution_status = "REVERTED".into();
    assert!(decode_receipt(&registry(), &r).unwrap().is_empty());
    r.execution_status = "SUCCEEDED".into();
    for e in &mut r.events {
        e.from_address = "0x123".into();
    }
    assert!(decode_receipt(&registry(), &r).unwrap().is_empty());
}
#[test]
fn modern_pool_event_omits_legacy_call_points() {
    let mut r = receipts().remove(1);
    let p = r
        .events
        .iter_mut()
        .find(|e| e.keys.first() == Some(&selector("PoolInitialized")))
        .unwrap();
    assert_eq!(p.data.len(), 10);
    p.data.pop();
    let l = decode_receipt(&registry(), &r).unwrap();
    assert!(l.iter().any(|l| l.kind == "PoolInitialized"));
}

#[test]
fn recent_mainnet_pool_receipt_uses_the_current_layout() {
    let r: Receipt =
        serde_json::from_str(include_str!("fixtures/modern-mainnet-receipt.json")).unwrap();
    let launches = decode_receipt(&registry(), &r).unwrap();
    assert!(
        launches
            .iter()
            .any(|l| l.kind == "PoolInitialized" && l.pool.is_some())
    );
}
#[test]
fn durable_dedup_outbox_and_reorg_correction() {
    let r = registry();
    let l = first_launch();
    let mut db = Store::open(Path::new(":memory:"), &r.fingerprint()).unwrap();
    let a = Alert {
        launch: l.clone(),
        message: "launch".into(),
        sink: "console".into(),
    };
    db.commit(l.block_number - 1, "0xabc", &[]).unwrap();
    db.commit(l.block_number, &l.block_hash, &[a]).unwrap();
    db.commit(
        l.block_number,
        &l.block_hash,
        &[Alert {
            launch: l.clone(),
            message: "duplicate".into(),
            sink: "console".into(),
        }],
    )
    .unwrap();
    assert_eq!(db.counts().unwrap(), (1, 0));
    assert_eq!(db.pending("console").unwrap().len(), 1);
    assert!(db.pending("telegram").unwrap().is_empty());
    db.delivered(&l.id).unwrap();
    assert_eq!(db.counts().unwrap(), (1, 1));
    db.rollback(l.block_number - 1).unwrap();
    assert!(!db.contains(&l.id).unwrap());
    assert_eq!(db.cursor().unwrap().unwrap().0, l.block_number - 1);
    let p = db.pending("console").unwrap();
    assert_eq!(p.len(), 1);
    assert!(p[0].message.contains("REORG CORRECTION"));
}
#[test]
fn missing_interfaces_are_not_a_safety_claim() {
    let l = first_launch();
    let d = enrich::Details {
        tokens: vec![TokenInfo {
            address: l.tokens[0].clone(),
            abi_available: true,
            ..Default::default()
        }],
        snapshot_block: l.block_number,
        ..Default::default()
    };
    let msg = enrich::message(&l, &d);
    assert!(msg.contains("behavior unknown"));
    assert!(msg.contains("Owner getter: unknown"));
    assert!(msg.contains("not a safety audit"));
}

async fn server(
    handler: impl Fn(Value) -> Value + Send + Sync + 'static,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handler = Arc::new(handler);
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let h = handler.clone();
            tokio::spawn(async move {
                let mut buffer = vec![];
                let mut chunk = [0u8; 4096];
                let (offset, length) = loop {
                    let Ok(n) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buffer.extend(&chunk[..n]);
                    if let Some(i) = buffer.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&buffer[..i]);
                        let len = header
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(str::trim)
                                    .and_then(|s| s.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if buffer.len() >= i + 4 + len {
                            break (i + 4, len);
                        }
                    }
                    if buffer.len() > 1024 * 1024 {
                        return;
                    }
                };
                let request: Value =
                    serde_json::from_slice(&buffer[offset..offset + length]).unwrap();
                let response = h(request);
                let body = serde_json::to_vec(&response).unwrap();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            });
        }
    });
    (format!("http://{address}"), task)
}
fn ok(result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":1,"result":result})
}
fn settings(url: String) -> Settings {
    Settings {
        rpc_url: url,
        ws_url: None,
        telegram: None,
        start_block: None,
        poll_secs: 5,
        batch: 100,
        db_path: Path::new(":memory:").into(),
    }
}

#[tokio::test]
async fn rpc_errors_suppress_provider_messages_and_endpoint_credentials() {
    let (url, task) =
        server(|_| json!({"error":{"code":-123,"message":"credential-leak-sentinel"}})).await;
    let rpc = Rpc::new(format!("{url}/credential-leak-sentinel")).unwrap();
    let error = rpc.head().await.unwrap_err().to_string();
    assert!(!error.contains("credential-leak-sentinel"));
    assert!(error.contains("-123"));
    assert!(
        rpc.call("starknet_addInvokeTransaction", json!({}))
            .await
            .is_err()
    );
    task.abort();
}
#[tokio::test]
async fn pagination_collects_every_page_and_rejects_repeated_tokens() {
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    let source = registry().sources[0].clone();
    let address = source.address.clone();
    let (url,task)=server(move |_|{let i=c.fetch_add(1,Ordering::SeqCst);if i==0 {ok(json!({"events":[{"block_number":10,"from_address":address,"transaction_hash":"0xa"}],"continuation_token":"next"}))}else{ok(json!({"events":[{"block_number":11,"from_address":address,"transaction_hash":"0xb"}]}))}}).await;
    assert_eq!(
        Rpc::new(url)
            .unwrap()
            .transactions(&source, 10, 11)
            .await
            .unwrap(),
        vec!["0xa", "0xb"]
    );
    task.abort();
    let (url, task) = server(|_| ok(json!({"events":[],"continuation_token":"repeated"}))).await;
    assert!(
        Rpc::new(url)
            .unwrap()
            .transactions(&source, 10, 11)
            .await
            .unwrap_err()
            .to_string()
            .contains("repeated")
    );
    task.abort();
}

fn pipeline_response(request: Value, r: &Receipt, reg: &Registry, fail_events: bool) -> Value {
    let method = request["method"].as_str().unwrap();
    let p = &request["params"];
    match method {
        "starknet_chainId" => ok(json!(reg.chain_id)),
        "starknet_blockNumber" => ok(json!(r.block_number)),
        "starknet_getClassHashAt" => ok(json!(
            reg.sources
                .iter()
                .find(|s| s.address == p["contract_address"])
                .map(|s| s.class_hash.clone())
                .unwrap_or_else(|| "0xabc".into())
        )),
        "starknet_getBlockWithTxHashes" => {
            ok(json!({"status":"ACCEPTED_ON_L2","block_hash":r.block_hash}))
        }
        "starknet_getEvents" if fail_events => json!({"error":{"code":-1,"message":"unavailable"}}),
        "starknet_getEvents" => {
            let address = p["filter"]["address"].as_str().unwrap();
            let events:Vec<_>=r.events.iter().filter(|e|e.from_address==address && e.keys.first()==Some(&selector("MemecoinCreated"))).map(|_|json!({"block_number":r.block_number,"from_address":address,"transaction_hash":r.transaction_hash})).collect();
            ok(json!({"events":events}))
        }
        "starknet_getTransactionReceipt" => ok(serde_json::to_value(r).unwrap()),
        "starknet_getClassAt" => ok(json!({"abi":[]})),
        "starknet_call" => {
            let entry = p["request"]["entry_point_selector"].as_str().unwrap();
            if entry == selector("name") || entry == selector("symbol") {
                ok(json!(["0x54455354"]))
            } else if entry == selector("total_supply") {
                ok(json!(["0x1", "0x0"]))
            } else {
                ok(json!(["0x0"]))
            }
        }
        _ => panic!("unexpected mock RPC method {method}"),
    }
}
#[tokio::test]
async fn accepted_range_is_atomic_durable_and_idempotent() {
    let receipt = receipts().remove(0);
    let block = receipt.block_number;
    let reg = registry();
    let r = receipt.clone();
    let copy = reg.clone();
    let (url, task) = server(move |request| pipeline_response(request, &r, &copy, false)).await;
    let mut engine = Engine::new(&settings(url), reg).unwrap();
    assert_eq!(engine.scan(block, block).await.unwrap(), 1);
    assert_eq!(engine.db.cursor().unwrap().unwrap().0, block);
    assert_eq!(engine.db.counts().unwrap(), (1, 0));
    assert_eq!(engine.scan(block, block).await.unwrap(), 0);
    engine.notifier.drain(&engine.db).await.unwrap();
    assert_eq!(engine.db.counts().unwrap(), (1, 1));
    task.abort();
}
#[tokio::test]
async fn failed_event_fetch_does_not_advance_checkpoint() {
    let receipt = receipts().remove(0);
    let block = receipt.block_number;
    let reg = registry();
    let copy = reg.clone();
    let (url, task) =
        server(move |request| pipeline_response(request, &receipt, &copy, true)).await;
    let mut engine = Engine::new(&settings(url), reg).unwrap();
    assert!(engine.scan(block, block).await.is_err());
    assert!(engine.db.cursor().unwrap().is_none());
    assert_eq!(engine.db.counts().unwrap(), (0, 0));
    task.abort();
}
#[tokio::test]
async fn wrong_chain_and_unknown_source_upgrade_stop_before_scanning() {
    let (url, task) = server(|_| ok(json!("0x534e5f5345504f4c4941"))).await;
    assert!(
        Rpc::new(url)
            .unwrap()
            .verify(&registry(), 10, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("chain ID mismatch")
    );
    task.abort();
    let (url, task) = server(|_| ok(json!("0x123"))).await;
    assert!(
        Rpc::new(url)
            .unwrap()
            .verify_hashes(&registry(), 10)
            .await
            .unwrap_err()
            .to_string()
            .contains("class changed")
    );
    task.abort();
}
