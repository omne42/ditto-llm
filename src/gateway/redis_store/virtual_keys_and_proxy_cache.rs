impl RedisStore {
    pub async fn load_virtual_keys(&self) -> Result<Vec<VirtualKeyConfig>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let key = self.key_virtual_keys();
        let raw_map: HashMap<String, String> = conn.hgetall(key).await?;
        let mut out: Vec<VirtualKeyConfig> = Vec::with_capacity(raw_map.len());
        for (_id, raw) in raw_map {
            out.push(serde_json::from_str(&raw)?);
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    pub async fn replace_virtual_keys(
        &self,
        keys: &[VirtualKeyConfig],
    ) -> Result<(), RedisStoreError> {
        let mut conn = self.connection().await?;
        let redis_key = self.key_virtual_keys();

        let mut pipe = redis::pipe();
        pipe.atomic().del(&redis_key);
        for key in keys {
            pipe.hset(&redis_key, &key.id, serde_json::to_string(key)?);
        }
        let _: () = pipe.query_async(&mut conn).await?;
        Ok(())
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn get_proxy_cache_response(
        &self,
        cache_key: &str,
    ) -> Result<Option<CachedProxyResponse>, RedisStoreError> {
        let mut conn = self.connection().await?;
        let redis_key = self.key_proxy_cache_response(cache_key);
        let raw: Option<Vec<u8>> = conn.get(redis_key).await?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let record: CachedProxyResponseRecord = match serde_json::from_slice(&raw) {
            Ok(record) => record,
            Err(_) => return Ok(None),
        };
        Ok(Some(record.into_cached()))
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn set_proxy_cache_response(
        &self,
        cache_key: &str,
        cached: &CachedProxyResponse,
        ttl_seconds: u64,
    ) -> Result<(), RedisStoreError> {
        if ttl_seconds == 0 {
            return Ok(());
        }

        let mut conn = self.connection().await?;
        let redis_key = self.key_proxy_cache_response(cache_key);
        let payload = serde_json::to_vec(&CachedProxyResponseRecord::from_cached(cached))?;
        let _: () = conn.set_ex(redis_key, payload, ttl_seconds).await?;
        Ok(())
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn delete_proxy_cache_response(
        &self,
        cache_key: &str,
    ) -> Result<u64, RedisStoreError> {
        let mut conn = self.connection().await?;
        let redis_key = self.key_proxy_cache_response(cache_key);
        let deleted: u64 = conn.del(redis_key).await?;
        Ok(deleted)
    }

    #[cfg(feature = "gateway-proxy-cache")]
    pub async fn clear_proxy_cache(&self) -> Result<u64, RedisStoreError> {
        let pattern = format!("{}:proxy_cache:*", self.prefix);
        let mut conn = self.connection().await?;
        let mut deleted = 0u64;

        let mut cursor = "0".to_string();
        loop {
            let (next_cursor, keys): (String, Vec<String>) = redis::cmd("SCAN")
                .arg(&cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(256)
                .query_async(&mut conn)
                .await?;

            for chunk in keys.chunks(128) {
                deleted = deleted.saturating_add(conn.del(chunk).await?);
            }

            if next_cursor == "0" {
                break;
            }
            cursor = next_cursor;
        }
        Ok(deleted)
    }
}
