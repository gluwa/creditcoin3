use axum::Json;
use serde_json::{json, Value};

pub async fn health_check() -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "database_connected": false,   // placeholder
        "proofs_stored": {
            "block_level": 0,
            "transaction_level": 0,
            "total": 0
        },
        "cache_hits": 0,
        "cache_misses": 0,
        "uptime_seconds": 0
    }))
}
