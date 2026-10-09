use crate::{
    config::{Registry, Source},
    decode::Receipt,
    encoding::{felt, selector},
};
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub struct Rpc {
    client: reqwest::Client,
    url: Arc<String>,
    pub calls: Arc<AtomicU64>,
}
impl Rpc {
    pub fn new(url: String) -> Result<Self> {
        crate::config::validate_url(&url, false)?;
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(25))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| anyhow::anyhow!("HTTP client setup failed"))?,
            url: Arc::new(url),
            calls: Arc::new(AtomicU64::new(0)),
        })
    }
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        ensure!(
            matches!(
                method,
                "starknet_chainId"
                    | "starknet_specVersion"
                    | "starknet_blockNumber"
                    | "starknet_getEvents"
                    | "starknet_getTransactionReceipt"
                    | "starknet_getClassHashAt"
                    | "starknet_getClassAt"
                    | "starknet_getBlockWithTxHashes"
                    | "starknet_call"
            ),
            "RPC method is not read-only allowlisted"
        );
        for attempt in 0..3 {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let response = self
                .client
                .post(self.url.as_str())
                .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
                .send()
                .await;
            let mut response = match response {
                Ok(r) => r,
                Err(_) if attempt < 2 => {
                    tokio::time::sleep(Duration::from_millis(500 * (1 << attempt))).await;
                    continue;
                }
                Err(_) => bail!("RPC transport failure for {method} (endpoint redacted)"),
            };
            let status = response.status();
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < 2 {
                tokio::time::sleep(Duration::from_millis(500 * (1 << attempt))).await;
                continue;
            }
            ensure!(
                status.is_success(),
                "RPC HTTP failure {} for {method} (endpoint redacted)",
                status.as_u16()
            );
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| anyhow::anyhow!("RPC body read failed for {method}"))?
            {
                ensure!(
                    body.len() + chunk.len() <= 16 * 1024 * 1024,
                    "RPC response exceeds size limit"
                );
                body.extend(chunk);
            }
            let value: Value = serde_json::from_slice(&body)
                .map_err(|_| anyhow::anyhow!("RPC response is not JSON for {method}"))?;
            if let Some(error) = value.get("error") {
                bail!(
                    "RPC error {} for {method} (provider message suppressed)",
                    error.get("code").and_then(Value::as_i64).unwrap_or(-1)
                );
            }
            return value
                .get("result")
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("RPC result missing for {method}"));
        }
        unreachable!()
    }
    pub async fn head(&self) -> Result<u64> {
        self.call("starknet_blockNumber", json!([]))
            .await?
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("invalid head number"))
    }
    pub async fn block(&self, n: u64) -> Result<Value> {
        self.call(
            "starknet_getBlockWithTxHashes",
            json!({"block_id":{"block_number":n}}),
        )
        .await
    }
    pub async fn hash(&self, n: u64) -> Result<String> {
        let b = self.block(n).await?;
        ensure!(
            matches!(
                b["status"].as_str(),
                Some("ACCEPTED_ON_L2" | "ACCEPTED_ON_L1")
            ),
            "block not accepted"
        );
        felt(
            b["block_hash"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("block hash missing"))?,
        )
    }
    pub async fn class_hash(&self, address: &str, n: u64) -> Result<String> {
        let h = self
            .call(
                "starknet_getClassHashAt",
                json!({"block_id":{"block_number":n},"contract_address":address}),
            )
            .await?;
        felt(
            h.as_str()
                .ok_or_else(|| anyhow::anyhow!("class hash missing"))?,
        )
    }
    pub async fn abi(&self, address: &str, n: u64) -> Result<Value> {
        let v = self
            .call(
                "starknet_getClassAt",
                json!({"block_id":{"block_number":n},"contract_address":address}),
            )
            .await?;
        if let Some(s) = v["abi"].as_str() {
            serde_json::from_str(s).map_err(|_| anyhow::anyhow!("invalid contract ABI"))
        } else {
            ensure!(v["abi"].is_array(), "missing contract ABI");
            Ok(v["abi"].clone())
        }
    }
    pub async fn read(
        &self,
        address: &str,
        method: &str,
        calldata: Vec<String>,
        n: u64,
    ) -> Result<Vec<String>> {
        let result=self.call("starknet_call",json!({"request":{"contract_address":address,"entry_point_selector":selector(method),"calldata":calldata},"block_id":{"block_number":n}})).await?;
        serde_json::from_value(result).map_err(|_| anyhow::anyhow!("invalid contract call result"))
    }
    pub async fn receipt(&self, hash: &str) -> Result<Receipt> {
        serde_json::from_value(
            self.call(
                "starknet_getTransactionReceipt",
                json!({"transaction_hash":hash}),
            )
            .await?,
        )
        .map_err(|_| anyhow::anyhow!("invalid accepted transaction receipt"))
    }
    pub async fn transactions(&self, s: &Source, from: u64, to: u64) -> Result<Vec<String>> {
        let mut token: Option<String> = None;
        let mut seen_tokens = std::collections::HashSet::new();
        let mut txs = std::collections::BTreeSet::new();
        for _ in 0..1000 {
            let mut filter = json!({"from_block":{"block_number":from},"to_block":{"block_number":to},"address":s.address,"keys":[s.selectors()],"chunk_size":100});
            if let Some(t) = &token {
                filter["continuation_token"] = json!(t);
            }
            let page = self
                .call("starknet_getEvents", json!({"filter":filter}))
                .await?;
            for e in page["events"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("invalid event page"))?
            {
                let n = e["block_number"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("missing event block"))?;
                ensure!(
                    n >= from && n <= to && s.matches(e["from_address"].as_str().unwrap_or("")),
                    "RPC returned event outside requested filter"
                );
                txs.insert(felt(
                    e["transaction_hash"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("missing event transaction"))?,
                )?);
            }
            token = page["continuation_token"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            match &token {
                None => return Ok(txs.into_iter().collect()),
                Some(t) => ensure!(
                    seen_tokens.insert(t.clone()),
                    "RPC repeated pagination token"
                ),
            }
        }
        bail!("event pagination limit exceeded; reduce BLOCK_BATCH_SIZE")
    }
    pub async fn verify(&self, r: &Registry, n: u64, latest: bool) -> Result<()> {
        let chain = self.call("starknet_chainId", json!([])).await?;
        ensure!(
            chain.as_str().and_then(|s| felt(s).ok()).as_deref() == Some(r.chain_id.as_str()),
            "RPC chain ID mismatch"
        );
        for s in &r.sources {
            let h = self.class_hash(&s.address, n).await?;
            ensure!(
                if latest {
                    h == s.class_hash
                } else {
                    s.accepts_class(&h)
                },
                "{} source class changed or historical class unsupported; review registry before continuing",
                s.name
            );
            let abi = self.abi(&s.address, n).await?;
            for expected in &s.event_layouts {
                ensure!(
                    abi.as_array().unwrap().iter().any(|a| a == expected
                        || s.legacy_event_layouts
                            .iter()
                            .any(|old| a == old && old["name"] == expected["name"])),
                    "{} event ABI differs from reviewed registry",
                    s.name
                );
            }
        }
        Ok(())
    }
    pub async fn verify_hashes(&self, r: &Registry, n: u64) -> Result<()> {
        for s in &r.sources {
            ensure!(
                s.accepts_class(&self.class_hash(&s.address, n).await?),
                "{} source class changed or historical class unsupported; review registry",
                s.name
            );
        }
        Ok(())
    }
}
