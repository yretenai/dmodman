use super::{CacheError, Cacheable, LocalFile};
use crate::Config;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::Arc;
use tokio::sync::RwLock;

use tokio::fs;

#[derive(Clone)]
pub struct LocalFileCache {
    map: Arc<RwLock<HashMap<u64, LocalFile>>>,
}

impl LocalFileCache {
    pub async fn new(config: &Config) -> Result<Self, CacheError> {
        let mut local_files: HashMap<u64, LocalFile> = HashMap::new();

        if let Ok(mut file_stream) = fs::read_dir(config.download_dir()).await {
            while let Some(f) = file_stream.next_entry().await? {
                if f.path().is_file() && f.path().extension().and_then(OsStr::to_str) == Some("json") {
                    if let Ok(lf) = LocalFile::load(f.path()).await {
                        local_files.insert(lf.file_id, lf);
                    }
                }
            }
        }

        Ok(Self {
            map: Arc::new(RwLock::new(local_files)),
        })
    }

    pub async fn push(&self, value: LocalFile) {
        self.map.write().await.insert(value.file_id, value);
    }

    pub async fn get(&self, key: u64) -> Option<LocalFile> {
        match self.map.read().await.get(&key) {
            Some(v) => Some(v.clone()),
            None => None,
        }
    }

    pub async fn items(&self) -> Vec<LocalFile> {
        self.map.read().await.values().cloned().collect()
    }
}
