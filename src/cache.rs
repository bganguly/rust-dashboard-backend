use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

struct Entry {
    value: Value,
    expires_at: Instant,
}

struct Inner {
    entries: HashMap<String, Entry>,
}

pub struct AggregatesCache {
    inner: RwLock<Inner>,
    max_size: usize,
    ttl: Duration,
}

impl AggregatesCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner { entries: HashMap::new() }),
            max_size: 50,
            ttl: Duration::from_secs(600),
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let guard = self.inner.read().unwrap();
        guard.entries.get(key).and_then(|e| {
            if Instant::now() < e.expires_at {
                Some(e.value.clone())
            } else {
                None
            }
        })
    }

    pub fn put(&self, key: String, value: Value) {
        let mut guard = self.inner.write().unwrap();
        if guard.entries.len() >= self.max_size {
            let oldest = guard
                .entries
                .iter()
                .min_by_key(|(_, e)| e.expires_at)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                guard.entries.remove(&k);
            }
        }
        guard.entries.insert(key, Entry {
            value,
            expires_at: Instant::now() + self.ttl,
        });
    }

    pub fn invalidate_all(&self) {
        let mut guard = self.inner.write().unwrap();
        guard.entries.clear();
    }
}

pub fn cache_key(from: &str, to: &str, top_n: i32) -> String {
    format!("{}|{}|{}", from, to, top_n)
}
