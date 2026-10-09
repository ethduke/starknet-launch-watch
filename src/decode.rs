use crate::{
    config::Registry,
    encoding::{cairo_text, felt, limb, require_len, selector, uint256},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RawEvent {
    pub from_address: String,
    pub keys: Vec<String>,
    pub data: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Receipt {
    pub transaction_hash: String,
    pub block_hash: String,
    pub block_number: u64,
    pub execution_status: String,
    pub finality_status: String,
    pub events: Vec<RawEvent>,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Pool {
    pub token0: String,
    pub token1: String,
    pub fee: String,
    pub tick_spacing: String,
    pub extension: String,
}
impl Pool {
    pub fn calldata(&self) -> Vec<String> {
        vec![
            self.token0.clone(),
            self.token1.clone(),
            self.fee.clone(),
            self.tick_spacing.clone(),
            self.extension.clone(),
        ]
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Launch {
    pub id: String,
    pub source: String,
    pub kind: String,
    pub block_number: u64,
    pub block_hash: String,
    pub tx_hash: String,
    pub event_index: usize,
    pub tokens: Vec<String>,
    pub owner: Option<String>,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub initial_supply: Option<String>,
    pub quote_token: Option<String>,
    pub exchange: Option<String>,
    pub pool: Option<Pool>,
}

pub fn decode_receipt(registry: &Registry, receipt: &Receipt) -> Result<Vec<Launch>> {
    if receipt.execution_status != "SUCCEEDED" {
        return Ok(vec![]);
    }
    ensure!(
        matches!(
            receipt.finality_status.as_str(),
            "ACCEPTED_ON_L2" | "ACCEPTED_ON_L1"
        ),
        "receipt is not accepted"
    );
    let mut result = vec![];
    let tx = felt(&receipt.transaction_hash)?;
    let block_hash = felt(&receipt.block_hash)?;
    for (index, e) in receipt.events.iter().enumerate() {
        let Some(source) = registry.sources.iter().find(|s| s.matches(&e.from_address)) else {
            continue;
        };
        let Some(key) = e.keys.first() else { continue };
        let key = felt(key)?;
        let Some(kind) = source.events.iter().find(|n| selector(n) == key) else {
            continue;
        };
        ensure!(e.keys.len() == 1, "unsupported indexed event layout");
        let mut l = Launch {
            id: format!("{}:{block_hash}:{tx}:{index}", registry.chain_id),
            source: source.name.clone(),
            kind: kind.clone(),
            block_number: receipt.block_number,
            block_hash: block_hash.clone(),
            tx_hash: tx.clone(),
            event_index: index,
            tokens: vec![],
            owner: None,
            name: None,
            symbol: None,
            initial_supply: None,
            quote_token: None,
            exchange: None,
            pool: None,
        };
        match kind.as_str() {
            "MemecoinCreated" => {
                require_len(&e.data, 6)?;
                l.owner = Some(felt(&e.data[0])?);
                l.name = cairo_text(&e.data[1..2]).ok();
                l.symbol = cairo_text(&e.data[2..3]).ok();
                l.initial_supply = Some(uint256(&e.data[3..5])?);
                l.tokens.push(felt(&e.data[5])?);
            }
            "MemecoinLaunched" => {
                require_len(&e.data, 3)?;
                l.tokens.push(felt(&e.data[0])?);
                l.quote_token = Some(felt(&e.data[1])?);
                l.exchange = cairo_text(&e.data[2..3]).ok();
            }
            "PoolInitialized" => {
                let legacy = e.data.len() == 10 && !source.legacy_event_layouts.is_empty();
                require_len(&e.data, if legacy { 10 } else { 9 })?;
                if legacy {
                    ensure!(
                        limb(&e.data[9])? <= num_bigint::BigUint::from(255u16),
                        "invalid legacy call points"
                    );
                }
                let pool = Pool {
                    token0: felt(&e.data[0])?,
                    token1: felt(&e.data[1])?,
                    fee: format!("0x{:x}", limb(&e.data[2])?),
                    tick_spacing: format!("0x{:x}", limb(&e.data[3])?),
                    extension: felt(&e.data[4])?,
                };
                ensure!(
                    crate::encoding::number(&pool.token0)? < crate::encoding::number(&pool.token1)?,
                    "invalid pool token order"
                );
                limb(&e.data[5])?;
                ensure!(
                    felt(&e.data[6])? == "0x0" || felt(&e.data[6])? == "0x1",
                    "invalid tick sign"
                );
                uint256(&e.data[7..9])?;
                l.tokens = vec![pool.token0.clone(), pool.token1.clone()];
                l.pool = Some(pool);
                l.exchange = Some("Ekubo".into());
            }
            _ => unreachable!(),
        }
        ensure!(l.tokens.iter().all(|t| t != "0x0"), "zero token address");
        result.push(l);
    }
    Ok(result)
}
