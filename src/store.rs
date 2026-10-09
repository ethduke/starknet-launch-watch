use crate::decode::Launch;
use anyhow::{Result, ensure};
use rusqlite::{Connection, params};

pub struct Alert {
    pub launch: Launch,
    pub message: String,
    pub sink: String,
}
pub struct Pending {
    pub id: String,
    pub message: String,
}
pub struct Store {
    conn: Connection,
}
impl Store {
    pub fn open(path: &std::path::Path, fingerprint: &str) -> Result<Self> {
        if let Some(p) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(p)?;
        }
        let s = Self {
            conn: Connection::open(path)?,
        };
        s.conn.busy_timeout(std::time::Duration::from_secs(5))?;
        s.conn.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS checkpoints (number INTEGER PRIMARY KEY,hash TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS events (id TEXT PRIMARY KEY,block INTEGER NOT NULL,hash TEXT NOT NULL,payload TEXT NOT NULL,message TEXT NOT NULL,sink TEXT NOT NULL,delivered INTEGER NOT NULL DEFAULT 0,canonical INTEGER NOT NULL DEFAULT 1);
            CREATE INDEX IF NOT EXISTS pending_delivery ON events(sink,delivered,block);")?;
        let existing = s
            .conn
            .query_row("SELECT value FROM meta WHERE key='registry'", [], |r| {
                r.get::<_, String>(0)
            });
        match existing {
            Ok(v) => ensure!(
                v == fingerprint,
                "registry changed; use a separate DATABASE_PATH to replay coverage safely"
            ),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                s.conn
                    .execute("INSERT INTO meta VALUES('registry',?1)", [fingerprint])?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(s)
    }
    pub fn cursor(&self) -> Result<Option<(u64, String)>> {
        let mut q = self
            .conn
            .prepare("SELECT number,hash FROM checkpoints ORDER BY number DESC LIMIT 1")?;
        let mut rows = q.query([])?;
        Ok(match rows.next()? {
            Some(r) => Some((r.get::<_, i64>(0)? as u64, r.get(1)?)),
            None => None,
        })
    }
    pub fn checkpoints(&self) -> Result<Vec<(u64, String)>> {
        let mut q = self
            .conn
            .prepare("SELECT number,hash FROM checkpoints ORDER BY number DESC LIMIT 128")?;
        Ok(
            q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?,
        )
    }
    pub fn contains(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM events WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?)
    }
    pub fn commit(&mut self, n: u64, hash: &str, alerts: &[Alert]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for a in alerts {
            tx.execute("INSERT OR IGNORE INTO events(id,block,hash,payload,message,sink) VALUES(?1,?2,?3,?4,?5,?6)",params![a.launch.id,i64::try_from(a.launch.block_number)?,a.launch.block_hash,serde_json::to_string(&a.launch)?,a.message,a.sink])?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO checkpoints VALUES(?1,?2)",
            params![i64::try_from(n)?, hash],
        )?;
        tx.execute("DELETE FROM checkpoints WHERE number NOT IN (SELECT number FROM checkpoints ORDER BY number DESC LIMIT 128)",[])?;
        tx.commit()?;
        Ok(())
    }
    pub fn pending(&self, sink: &str) -> Result<Vec<Pending>> {
        let mut q=self.conn.prepare("SELECT id,message FROM events WHERE delivered=0 AND sink=?1 ORDER BY block,id LIMIT 100")?;
        Ok(q.query_map([sink], |r| {
            Ok(Pending {
                id: r.get(0)?,
                message: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
    pub fn delivered(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE events SET delivered=1 WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn rollback(&mut self, n: u64) -> Result<()> {
        let n = i64::try_from(n)?;
        let corrections = {
            let mut q = self.conn.prepare(
                "SELECT id,sink,payload FROM events WHERE block>?1 AND delivered=1 AND canonical=1",
            )?;
            q.query_map([n], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM events WHERE block>?1 AND canonical=1", [n])?;
        tx.execute("DELETE FROM checkpoints WHERE number>?1", [n])?;
        for (id, sink, payload) in corrections {
            let l: Launch = serde_json::from_str(&payload)?;
            let message = format!(
                "REORG CORRECTION: withdraw the earlier {} alert at block {}.\nTx: https://starkscan.co/tx/{}\nThis event is no longer on the observed canonical chain.",
                l.kind, l.block_number, l.tx_hash
            );
            tx.execute("INSERT OR IGNORE INTO events(id,block,hash,payload,message,sink,canonical) VALUES(?1,?2,'',?3,?4,?5,0)",params![format!("reorg:{id}"),n,payload,message,sink])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn counts(&self) -> Result<(u64, u64)> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*),COALESCE(SUM(delivered),0) FROM events",
            [],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )?)
    }
}
