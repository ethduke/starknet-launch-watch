use crate::{
    config::{Registry, Settings},
    decode::decode_receipt,
    enrich,
    notify::Notifier,
    rpc::Rpc,
    store::{Alert, Store},
};
use anyhow::{Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

pub struct Engine {
    pub rpc: Rpc,
    pub registry: Registry,
    pub db: Store,
    pub notifier: Notifier,
    pub batch: u64,
}
impl Engine {
    pub fn new(settings: &Settings, registry: Registry) -> Result<Self> {
        Ok(Self {
            rpc: Rpc::new(settings.rpc_url.clone())?,
            db: Store::open(&settings.db_path, &registry.fingerprint())?,
            notifier: Notifier::new(settings.telegram.clone())?,
            registry,
            batch: settings.batch,
        })
    }
    pub async fn reconcile(&mut self) -> Result<()> {
        let points = self.db.checkpoints()?;
        if points.is_empty() {
            return Ok(());
        }
        let head = self.rpc.head().await?;
        for (i, (n, h)) in points.iter().enumerate() {
            if *n > head {
                continue;
            }
            if self.rpc.hash(*n).await? == *h {
                if i > 0 {
                    self.db.rollback(*n)?;
                    eprintln!("Canonical chain changed; rewound checkpoint to block {n}");
                }
                return Ok(());
            }
        }
        bail!("no canonical checkpoint in retained history; stop and replay into a fresh database")
    }
    pub async fn scan(&mut self, from: u64, to: u64) -> Result<usize> {
        ensure!(to >= from && to - from < self.batch, "invalid scan range");
        let anchor = self.rpc.hash(to).await?;
        self.rpc.verify_hashes(&self.registry, to).await?;
        let mut txs = BTreeSet::new();
        for s in &self.registry.sources {
            txs.extend(self.rpc.transactions(s, from, to).await?);
        }
        ensure!(
            txs.len() <= 5000,
            "too many candidate transactions; reduce batch size"
        );
        let mut receipts = vec![];
        for tx in txs {
            let r = self.rpc.receipt(&tx).await?;
            ensure!(
                crate::encoding::felt(&r.transaction_hash)? == tx
                    && r.block_number >= from
                    && r.block_number <= to,
                "receipt outside requested scan range"
            );
            receipts.push(r);
        }
        receipts.sort_by_key(|r| (r.block_number, r.transaction_hash.clone()));
        let mut block_hashes: HashMap<u64, String> = HashMap::new();
        let mut alerts = vec![];
        let mut token_cache: HashMap<(String, u64), enrich::TokenInfo> = HashMap::new();
        let core = self
            .registry
            .sources
            .iter()
            .find(|s| s.name == "ekubo")
            .map(|s| s.address.clone());
        for receipt in receipts {
            let h = if let Some(h) = block_hashes.get(&receipt.block_number) {
                h.clone()
            } else {
                let h = self.rpc.hash(receipt.block_number).await?;
                self.rpc
                    .verify_hashes(&self.registry, receipt.block_number)
                    .await?;
                block_hashes.insert(receipt.block_number, h.clone());
                h
            };
            ensure!(
                crate::encoding::felt(&receipt.block_hash)? == h,
                "receipt no longer canonical; retry range"
            );
            for launch in decode_receipt(&self.registry, &receipt)? {
                if self.db.contains(&launch.id)? {
                    continue;
                }
                let mut details = enrich::Details {
                    snapshot_block: launch.block_number,
                    ..Default::default()
                };
                for address in &launch.tokens {
                    let key = (address.clone(), launch.block_number);
                    let token = if let Some(t) = token_cache.get(&key) {
                        t.clone()
                    } else {
                        let t = enrich::token(&self.rpc, address, launch.block_number).await;
                        token_cache.insert(key, t.clone());
                        t
                    };
                    details.tokens.push(token);
                }
                if let (Some(core), Some(p)) = (&core, &launch.pool) {
                    details.active_liquidity_raw =
                        enrich::liquidity(&self.rpc, core, p, launch.block_number).await;
                }
                let message = enrich::message(&launch, &details);
                alerts.push(Alert {
                    launch,
                    message,
                    sink: self.notifier.sink().into(),
                });
            }
        }
        ensure!(
            self.rpc.hash(to).await? == anchor,
            "chain changed during enrichment; retry range"
        );
        self.db.commit(to, &anchor, &alerts)?;
        Ok(alerts.len())
    }
    pub async fn catch_up(&mut self, start: u64, stop: u64) -> Result<()> {
        self.reconcile().await?;
        let mut from = self.db.cursor()?.map(|(n, _)| n + 1).unwrap_or(start);
        while from <= stop {
            let to = (from.saturating_add(self.batch - 1)).min(stop);
            let n = self.scan(from, to).await?;
            eprintln!("Processed blocks {from}..{to}: {n} new observations");
            self.notifier.drain(&self.db).await?;
            from = to + 1;
        }
        self.notifier.drain(&self.db).await?;
        Ok(())
    }
    pub async fn watch(&mut self, settings: &Settings, once: bool) -> Result<()> {
        let head = self.rpc.head().await?;
        self.rpc.verify(&self.registry, head, true).await?;
        // Establish an anchor even when there are no launches, so reconnects cannot skip blocks.
        let start = settings.start_block.unwrap_or(head + 1);
        if self.db.cursor()?.is_none() && start > 0 {
            ensure!(
                start <= head + 1,
                "START_BLOCK is ahead of the accepted chain"
            );
            let previous = start - 1;
            self.db
                .commit(previous, &self.rpc.hash(previous).await?, &[])?;
        }
        let (wake_tx, mut wake_rx) = mpsc::channel(1);
        let ws_task = settings.ws_url.clone().map(|url| {
            let registry = self.registry.clone();
            tokio::spawn(async move {
                websocket_wakes(url, registry, wake_tx).await;
            })
        });
        let mut poll = tokio::time::interval(Duration::from_secs(settings.poll_secs));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let step = async {
                let head = self.rpc.head().await?;
                self.catch_up(start, head).await
            }
            .await;
            if let Err(e) = step {
                eprintln!("Scan paused: {e}");
                if once {
                    if let Some(t) = &ws_task {
                        t.abort();
                    }
                    return Err(e);
                }
            }
            if once {
                break;
            }
            tokio::select! {
                _=poll.tick()=>{},
                Some(_)=wake_rx.recv()=>{},
                _=tokio::signal::ctrl_c()=>{eprintln!("Stopping; checkpoint and pending alerts saved");break;}
            }
        }
        if let Some(t) = ws_task {
            t.abort();
        }
        Ok(())
    }
}

async fn websocket_wakes(url: String, registry: Registry, tx: mpsc::Sender<()>) {
    let addresses: Vec<_> = registry.sources.iter().map(|s| s.address.clone()).collect();
    let selectors: Vec<_> = registry
        .sources
        .iter()
        .flat_map(|s| s.selectors())
        .collect();
    let subscription = json!({"jsonrpc":"2.0","id":1,"method":"starknet_subscribeEvents","params":{
        "from_address":addresses,"keys":[selectors],"finality_status":"ACCEPTED_ON_L2"}})
    .to_string();
    loop {
        let connect = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async_with_config(
                &url,
                Some(
                    tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
                        .max_message_size(Some(256 * 1024))
                        .max_frame_size(Some(256 * 1024)),
                ),
                false,
            ),
        )
        .await;
        if let Ok(Ok((mut socket, _))) = connect
            && socket
                .send(Message::Text(subscription.clone().into()))
                .await
                .is_ok()
        {
            eprintln!("WebSocket connected; HTTP polling and recovery remain enabled");
            let _ = tx.try_send(());
            loop {
                match tokio::time::timeout(Duration::from_secs(45), socket.next()).await {
                    Ok(Some(Ok(Message::Text(text)))) if text.len() <= 256 * 1024 => {
                        let v = serde_json::from_str::<serde_json::Value>(&text);
                        match v {
                            Ok(v) if v.get("error").is_some() => break,
                            Ok(v) if v["method"] == "starknet_subscriptionEvents" => {
                                let _ = tx.try_send(());
                            }
                            _ => {}
                        }
                    }
                    Ok(Some(Ok(Message::Ping(p)))) => {
                        if socket.send(Message::Pong(p)).await.is_err() {
                            break;
                        }
                    }
                    Ok(Some(Ok(Message::Pong(_)))) => {}
                    Err(_) => {
                        if socket.send(Message::Ping(vec![].into())).await.is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }
        eprintln!(
            "WebSocket unavailable; HTTP polling continues (endpoint and transport details redacted)"
        );
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}
