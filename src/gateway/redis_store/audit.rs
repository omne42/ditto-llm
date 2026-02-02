impl RedisStore {
    pub async fn append_audit_log(
        &self,
        kind: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let kind = kind.into();
        let ts_ms = now_millis_u64();
        let id: i64 = conn.incr(self.key_audit_seq(), 1).await?;
        let member = format!("{id:020}");
        let record = AuditLogRecord {
            id,
            ts_ms,
            kind,
            payload,
        };
        let serialized = serde_json::to_string(&record)?;

        let record_key = self.key_audit_record(&member);
        let idx_key = self.key_audit_by_ts();

        let retention_secs = self.audit_retention_secs;
        let mut pipe = redis::pipe();
        pipe.atomic();
        if let Some(retention_secs) = retention_secs {
            pipe.cmd("SET")
                .arg(&record_key)
                .arg(&serialized)
                .arg("EX")
                .arg(retention_secs.max(1));
        } else {
            pipe.set(&record_key, &serialized);
        }
        pipe.zadd(&idx_key, member, ts_ms);
        if let Some(retention_secs) = retention_secs {
            let retention_ms = retention_secs.saturating_mul(1000);
            let cutoff_ms = ts_ms.saturating_sub(retention_ms);
            pipe.cmd("ZREMRANGEBYSCORE")
                .arg(&idx_key)
                .arg("-inf")
                .arg(cutoff_ms);
        }
        let _: () = pipe.query_async(&mut conn).await?;
        Ok(())
    }

    pub async fn list_audit_logs(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let idx_key = self.key_audit_by_ts();
        let limit = limit.clamp(1, 1000);

        let members: Vec<String> = if let Some(since) = since_ts_ms {
            redis::cmd("ZREVRANGEBYSCORE")
                .arg(&idx_key)
                .arg("+inf")
                .arg(since)
                .arg("LIMIT")
                .arg(0)
                .arg(limit)
                .query_async(&mut conn)
                .await?
        } else {
            redis::cmd("ZREVRANGEBYSCORE")
                .arg(&idx_key)
                .arg("+inf")
                .arg("-inf")
                .arg("LIMIT")
                .arg(0)
                .arg(limit)
                .query_async(&mut conn)
                .await?
        };

        let mut out = Vec::with_capacity(members.len());
        for member in members {
            let record_key = self.key_audit_record(&member);
            let raw: Option<String> = conn.get(record_key).await?;
            let Some(raw) = raw else {
                continue;
            };
            out.push(serde_json::from_str(&raw)?);
        }
        Ok(out)
    }

    pub async fn list_audit_logs_window(
        &self,
        limit: usize,
        since_ts_ms: Option<u64>,
        before_ts_ms: Option<u64>,
    ) -> Result<Vec<AuditLogRecord>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let idx_key = self.key_audit_by_ts();
        let limit = limit.clamp(1, 10_000);

        let max = before_ts_ms
            .map(|value| format!("({value}"))
            .unwrap_or_else(|| "+inf".to_string());
        let min = since_ts_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-inf".to_string());

        let members: Vec<String> = redis::cmd("ZREVRANGEBYSCORE")
            .arg(&idx_key)
            .arg(max)
            .arg(min)
            .arg("LIMIT")
            .arg(0)
            .arg(limit)
            .query_async(&mut conn)
            .await?;

        let mut out = Vec::with_capacity(members.len());
        for member in members {
            let record_key = self.key_audit_record(&member);
            let raw: Option<String> = conn.get(record_key).await?;
            let Some(raw) = raw else {
                continue;
            };
            out.push(serde_json::from_str(&raw)?);
        }
        Ok(out)
    }

    pub async fn list_cost_ledgers(&self) -> Result<Vec<CostLedgerRecord>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let mut key_ids: Vec<String> = conn.smembers(self.key_cost_keys()).await?;
        key_ids.sort();

        let mut out = Vec::with_capacity(key_ids.len());
        for key_id in key_ids {
            let ledger_key = self.key_cost_ledger(&key_id);
            let raw: HashMap<String, String> = conn.hgetall(ledger_key).await?;
            let spent_usd_micros = raw
                .get("spent_usd_micros")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let reserved_usd_micros = raw
                .get("reserved_usd_micros")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let updated_at_ms = raw
                .get("updated_at_ms")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            out.push(CostLedgerRecord {
                key_id,
                spent_usd_micros,
                reserved_usd_micros,
                updated_at_ms,
            });
        }
        Ok(out)
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn now_millis_u64() -> u64 {
    if now_millis() <= 0 {
        0
    } else {
        now_millis() as u64
    }
}

fn tokens_to_i64(tokens: u64) -> i64 {
    if tokens > i64::MAX as u64 {
        i64::MAX
    } else {
        tokens as i64
    }
}
