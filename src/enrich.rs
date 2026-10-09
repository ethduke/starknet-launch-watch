use crate::{
    decode::{Launch, Pool},
    encoding::{cairo_text, display_units, felt, number, uint256},
    rpc::Rpc,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct TokenInfo {
    pub address: String,
    pub class_hash: Option<String>,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: Option<u32>,
    pub supply: Option<String>,
    pub owner: Option<String>,
    pub paused: Option<bool>,
    pub exposed_controls: Vec<String>,
    pub abi_available: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Details {
    pub tokens: Vec<TokenInfo>,
    pub active_liquidity_raw: Option<String>,
    pub snapshot_block: u64,
}

pub fn functions(abi: &Value) -> Vec<String> {
    let mut names = vec![];
    if let Some(items) = abi.as_array() {
        for item in items {
            if item["type"] == "function"
                && let Some(n) = item["name"].as_str()
            {
                names.push(n.to_owned());
            }
            if item["type"] == "interface" {
                names.extend(functions(&item["items"]));
            }
        }
    }
    names
}
async fn read_any(rpc: &Rpc, address: &str, names: &[&str], n: u64) -> Option<Vec<String>> {
    for name in names {
        if let Ok(v) = rpc.read(address, name, vec![], n).await {
            return Some(v);
        }
    }
    None
}
pub async fn token(rpc: &Rpc, address: &str, n: u64) -> TokenInfo {
    let mut t = TokenInfo {
        address: address.into(),
        ..Default::default()
    };
    t.class_hash = rpc.class_hash(address, n).await.ok();
    if let Ok(abi) = rpc.abi(address, n).await {
        t.abi_available = true;
        let names = functions(&abi);
        let controls = [
            "mint",
            "mint_to",
            "mintTo",
            "pause",
            "unpause",
            "upgrade",
            "upgrade_to",
            "upgradeTo",
            "replace_class",
            "set_admin",
            "setAdmin",
            "transfer_ownership",
            "transferOwnership",
            "grant_role",
            "grantRole",
            "blacklist",
            "set_blacklist",
            "set_fee",
            "set_tax",
        ];
        t.exposed_controls = names
            .into_iter()
            .filter(|s| controls.contains(&s.as_str()))
            .collect();
    }
    t.name = match read_any(rpc, address, &["name"], n).await {
        Some(v) => cairo_text(&v).ok(),
        None => None,
    };
    t.symbol = match read_any(rpc, address, &["symbol"], n).await {
        Some(v) => cairo_text(&v).ok(),
        None => None,
    };
    t.decimals = read_any(rpc, address, &["decimals"], n)
        .await
        .and_then(|v| match v.as_slice() {
            [x] => number(x).ok()?.to_string().parse::<u32>().ok(),
            _ => None,
        })
        .filter(|n| *n <= 36);
    t.supply = read_any(rpc, address, &["total_supply", "totalSupply"], n)
        .await
        .and_then(|v| uint256(&v).ok());
    t.owner = read_any(rpc, address, &["owner", "get_owner", "getOwner"], n)
        .await
        .and_then(|v| match v.as_slice() {
            [s] => felt(s).ok(),
            _ => None,
        });
    t.paused = read_any(rpc, address, &["paused", "is_paused"], n)
        .await
        .and_then(|v| match v.as_slice() {
            [x] => match felt(x).ok()?.as_str() {
                "0x0" => Some(false),
                "0x1" => Some(true),
                _ => None,
            },
            _ => None,
        });
    t
}
pub async fn liquidity(rpc: &Rpc, core: &str, p: &Pool, n: u64) -> Option<String> {
    let v = rpc
        .read(core, "get_pool_liquidity", p.calldata(), n)
        .await
        .ok()?;
    if let [x] = v.as_slice() {
        crate::encoding::limb(x).ok().map(|n| n.to_string())
    } else {
        None
    }
}
pub async fn details(rpc: &Rpc, launch: &Launch, core: Option<&str>) -> Details {
    let mut d = Details {
        snapshot_block: launch.block_number,
        ..Default::default()
    };
    for a in &launch.tokens {
        d.tokens.push(token(rpc, a, launch.block_number).await);
    }
    if let (Some(core), Some(pool)) = (core, &launch.pool) {
        d.active_liquidity_raw = liquidity(rpc, core, pool, launch.block_number).await;
    }
    d
}
pub fn message(l: &Launch, d: &Details) -> String {
    let title = match l.kind.as_str() {
        "MemecoinCreated" => "TOKEN CREATED (trading may not be open)",
        "MemecoinLaunched" => "LAUNCH OBSERVED",
        _ => "NEW EKUBO POOL (token age unknown)",
    };
    let mut lines = vec![format!(
        "{title}\nSource: {} | Block: {}",
        l.source, l.block_number
    )];
    for t in &d.tokens {
        lines.push(format!(
            "\n{} ({})\nToken: {}",
            t.name
                .as_deref()
                .or(l.name.as_deref())
                .unwrap_or("Unknown name"),
            t.symbol.as_deref().or(l.symbol.as_deref()).unwrap_or("?"),
            t.address
        ));
        if let Some(h) = &t.class_hash {
            lines.push(format!("Class: {h}"));
        }
        let supply = t.supply.as_ref().or(l.initial_supply.as_ref());
        if let Some(raw) = supply {
            lines.push(match t.decimals {
                Some(dec) => format!(
                    "Supply: {}",
                    display_units(raw, dec).unwrap_or_else(|_| "unknown".into())
                ),
                None => format!("Supply: {raw} raw units (decimals unknown)"),
            });
        }
        lines.push(format!(
            "Owner getter: {}",
            t.owner.as_deref().unwrap_or("unknown")
        ));
        lines.push(format!(
            "Paused getter: {}",
            t.paused
                .map(|x| x.to_string())
                .unwrap_or_else(|| "unknown".into())
        ));
        let control_text = if !t.abi_available {
            "unknown (ABI unavailable)".into()
        } else if t.exposed_controls.is_empty() {
            "none of the checked names observed; behavior unknown".into()
        } else {
            t.exposed_controls.join(", ")
        };
        lines.push(format!("Control entrypoints: {control_text}"));
        lines.push(format!("View: https://starkscan.co/contract/{}", t.address));
    }
    if let Some(p) = &l.pool {
        let pct = crate::encoding::limb(&p.fee)
            .ok()
            .and_then(|v| v.to_string().parse::<f64>().ok())
            .map(|v| v / (2f64.powi(128)) * 100.0);
        lines.push(format!(
            "\nPool fee: {} | Extension: {}",
            pct.map(|n| format!("{n:.4}%"))
                .unwrap_or_else(|| "unknown".into()),
            p.extension
        ));
        lines.push(format!(
            "Active liquidity L: {} (raw protocol units; not USD)",
            d.active_liquidity_raw.as_deref().unwrap_or("unknown")
        ));
    } else {
        lines.push("\nPool liquidity: unknown (no pool key in this factory event)".into());
    }
    lines.push("ABI/getter observations are not a safety audit. USD depth, liquidity locks and executable quotes are not yet verified.".into());
    lines.push(format!(
        "Snapshot: block {}\nTx: https://starkscan.co/tx/{}",
        d.snapshot_block, l.tx_hash
    ));
    lines.push(
        "Trade: https://app.avnu.fi/ (paste the token address; connect and sign with your wallet)"
            .into(),
    );
    lines.join("\n").chars().take(3900).collect()
}
