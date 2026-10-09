use crate::encoding::{felt, selector};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{env, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct Registry {
    pub chain_id: String,
    pub verified_at: String,
    pub sources: Vec<Source>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub address: String,
    pub class_hash: String,
    pub events: Vec<String>,
    pub event_layouts: Vec<Value>,
    #[serde(default)]
    pub compatible_class_hashes: Vec<String>,
    #[serde(default)]
    pub legacy_event_layouts: Vec<Value>,
}
impl Source {
    pub fn matches(&self, address: &str) -> bool {
        felt(address).is_ok_and(|a| a == self.address)
    }
    pub fn selectors(&self) -> Vec<String> {
        self.events.iter().map(|e| selector(e)).collect()
    }
    pub fn accepts_class(&self, hash: &str) -> bool {
        felt(hash).is_ok_and(|h| h == self.class_hash || self.compatible_class_hashes.contains(&h))
    }
}
impl Registry {
    pub fn load(path: &str) -> Result<Self> {
        let mut r: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        r.chain_id = felt(&r.chain_id)?;
        ensure!(
            r.chain_id == "0x534e5f4d41494e",
            "this registry supports Starknet mainnet only"
        );
        ensure!(
            !r.sources.is_empty() && r.sources.len() <= 20,
            "invalid source count"
        );
        let mut addresses = std::collections::HashSet::new();
        for s in &mut r.sources {
            ensure!(
                matches!(s.name.as_str(), "unruggable" | "ekubo"),
                "unsupported source adapter"
            );
            s.address = felt(&s.address)?;
            ensure!(
                s.address != "0x0" && addresses.insert(s.address.clone()),
                "invalid or duplicate source address"
            );
            s.class_hash = felt(&s.class_hash)?;
            for h in &mut s.compatible_class_hashes {
                *h = felt(h)?;
            }
            let supported: &[&str] = if s.name == "ekubo" {
                &["PoolInitialized"]
            } else {
                &["MemecoinCreated", "MemecoinLaunched"]
            };
            ensure!(
                !s.events.is_empty() && s.events.iter().all(|e| supported.contains(&e.as_str())),
                "unsupported event adapter"
            );
        }
        Ok(r)
    }
    pub fn fingerprint(&self) -> String {
        selector(&serde_json::to_string(self).unwrap())
    }
}

// Deliberately no Debug/Serialize implementation: endpoints may embed credentials.
pub struct Settings {
    pub rpc_url: String,
    pub ws_url: Option<String>,
    pub telegram: Option<(String, String)>,
    pub start_block: Option<u64>,
    pub poll_secs: u64,
    pub batch: u64,
    pub db_path: PathBuf,
}
fn optional(name: &str) -> Option<String> {
    env::var(name).ok().filter(|s| !s.is_empty())
}
fn integer(name: &str, default: u64) -> Result<u64> {
    match optional(name) {
        Some(s) => s
            .parse()
            .map_err(|_| anyhow::anyhow!("{name} must be an integer")),
        None => Ok(default),
    }
}
impl Settings {
    pub fn load(console: bool) -> Result<Self> {
        let rpc_url = optional("STARKNET_RPC_URL")
            .ok_or_else(|| anyhow::anyhow!("set STARKNET_RPC_URL in local .env"))?;
        validate_url(&rpc_url, false)?;
        let ws_url = optional("STARKNET_RPC_WS_URL");
        if let Some(s) = &ws_url {
            validate_url(s, true)?;
        }
        let token = optional("TELEGRAM_BOT_TOKEN");
        let chat = optional("TELEGRAM_CHAT_ID");
        let telegram = if console {
            None
        } else {
            ensure!(
                token.is_some() == chat.is_some(),
                "set both TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID, or neither"
            );
            if let (Some(t), Some(c)) = (token, chat) {
                ensure!(
                    t.len() <= 200
                        && t.chars()
                            .all(|x| x.is_ascii_alphanumeric() || "_: -".contains(x))
                        && !t.contains(' '),
                    "invalid Telegram token format"
                );
                ensure!(
                    c.len() <= 100
                        && c.chars()
                            .all(|x| x.is_ascii_alphanumeric() || "_@-".contains(x)),
                    "invalid Telegram destination"
                );
                Some((t, c))
            } else {
                None
            }
        };
        let poll_secs = integer("POLL_INTERVAL_SECS", 5)?;
        let batch = integer("BLOCK_BATCH_SIZE", 100)?;
        ensure!(
            (1..=300).contains(&poll_secs),
            "poll interval must be 1..300 seconds"
        );
        ensure!(
            (1..=1000).contains(&batch),
            "block batch size must be 1..1000"
        );
        Ok(Self {
            rpc_url,
            ws_url,
            telegram,
            start_block: optional("START_BLOCK")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| anyhow::anyhow!("START_BLOCK must be an integer"))?,
            poll_secs,
            batch,
            db_path: optional("DATABASE_PATH")
                .unwrap_or_else(|| "data/watch.sqlite".into())
                .into(),
        })
    }
}
pub fn validate_url(s: &str, websocket: bool) -> Result<()> {
    let u =
        reqwest::Url::parse(s).map_err(|_| anyhow::anyhow!("invalid endpoint URL (redacted)"))?;
    let local = matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    let secure = if websocket { "wss" } else { "https" };
    let plain = if websocket { "ws" } else { "http" };
    ensure!(
        u.scheme() == secure || (local && u.scheme() == plain),
        "endpoint must use TLS (localhost exempt)"
    );
    ensure!(
        u.fragment().is_none(),
        "endpoint URL must not contain a fragment"
    );
    Ok(())
}
