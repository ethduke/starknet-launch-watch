use crate::store::Store;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::time::Duration;

pub struct Notifier {
    telegram: Option<(String, String)>,
    client: reqwest::Client,
}
impl Notifier {
    pub fn new(telegram: Option<(String, String)>) -> Result<Self> {
        Ok(Self {
            telegram,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| anyhow::anyhow!("notification client setup failed"))?,
        })
    }
    pub fn sink(&self) -> &str {
        if self.telegram.is_some() {
            "telegram"
        } else {
            "console"
        }
    }
    pub async fn drain(&self, db: &Store) -> Result<()> {
        loop {
            let pending = db.pending(self.sink())?;
            if pending.is_empty() {
                return Ok(());
            }
            for p in pending {
                if let Some((token, chat)) = &self.telegram {
                    let mut request = json!({"chat_id":chat,"text":p.message,"link_preview_options":{"is_disabled":true}});
                    if !p.id.starts_with("reorg:") {
                        request["reply_markup"] = json!({"inline_keyboard":[[{"text":"Trade on AVNU","url":"https://app.avnu.fi/"}]]});
                    }
                    // Telegram embeds the credential in its URL. Never format its errors.
                    let response=self.client.post(format!("https://api.telegram.org/bot{token}/sendMessage"))
                        .json(&request)
                        .send().await.map_err(|_| anyhow::anyhow!("Telegram transport failure (details redacted); alert remains queued"))?;
                    ensure!(
                        response.status().is_success(),
                        "Telegram HTTP failure {}; alert remains queued",
                        response.status().as_u16()
                    );
                    let body: Value = response.json().await.map_err(|_| {
                        anyhow::anyhow!("invalid Telegram response (details suppressed)")
                    })?;
                    ensure!(
                        body["ok"] == true,
                        "Telegram rejected delivery (details suppressed); alert remains queued"
                    );
                    tokio::time::sleep(Duration::from_millis(1100)).await;
                } else {
                    println!("{}\n", p.message);
                }
                db.delivered(&p.id)?;
            }
        }
    }
}
