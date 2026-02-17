#[cfg(feature = "gateway")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::BufRead;

    let mut args = std::env::args().skip(1);
    let usage = "usage: ditto-audit-verify --input PATH|-";

    let mut input: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = Some(args.next().ok_or("missing value for --input")?),
            "--help" | "-h" => {
                println!("{usage}");
                return Ok(());
            }
            other => return Err(format!("unknown arg: {other}").into()),
        }
    }

    let input = input.ok_or(usage)?;

    #[derive(Debug, serde::Deserialize)]
    struct AuditExportRecord {
        id: i64,
        ts_ms: u64,
        kind: String,
        payload: serde_json::Value,
        #[serde(default)]
        prev_hash: Option<String>,
        hash: String,
    }

    fn hex_lower(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len().saturating_mul(2));
        for byte in bytes {
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        out
    }

    fn audit_chain_hash(
        prev_hash: Option<&str>,
        record: &ditto_llm::gateway::AuditLogRecord,
    ) -> String {
        use sha2::Digest as _;

        let mut hasher = sha2::Sha256::new();
        if let Some(prev_hash) = prev_hash {
            hasher.update(prev_hash.as_bytes());
        }
        hasher.update(b"\n");
        if let Ok(serialized) = serde_json::to_vec(record) {
            hasher.update(&serialized);
        }
        hex_lower(&hasher.finalize())
    }

    let reader: Box<dyn BufRead> = if input == "-" {
        Box::new(std::io::BufReader::new(std::io::stdin()))
    } else {
        let file = std::fs::File::open(&input)?;
        Box::new(std::io::BufReader::new(file))
    };

    let mut prev_hash: Option<String> = None;
    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let record: AuditExportRecord = serde_json::from_str(line)?;

        let expected_prev = prev_hash.as_deref().unwrap_or("");
        let got_prev = record.prev_hash.as_deref().unwrap_or("");
        if expected_prev != got_prev {
            return Err(format!(
                "hash chain mismatch at line {}: prev_hash expected {} got {}",
                line_no + 1,
                expected_prev,
                got_prev
            )
            .into());
        }

        let base = ditto_llm::gateway::AuditLogRecord {
            id: record.id,
            ts_ms: record.ts_ms,
            kind: record.kind,
            payload: record.payload,
        };
        let expected_hash = audit_chain_hash(prev_hash.as_deref(), &base);
        if record.hash != expected_hash {
            return Err(format!(
                "hash mismatch at line {}: expected {} got {}",
                line_no + 1,
                expected_hash,
                record.hash
            )
            .into());
        }
        prev_hash = Some(record.hash);
    }

    println!("ok");
    Ok(())
}

#[cfg(not(feature = "gateway"))]
fn main() {
    eprintln!("ditto-audit-verify requires `--features gateway`");
    std::process::exit(2);
}
