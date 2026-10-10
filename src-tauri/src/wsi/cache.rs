use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

/// Bounded cache capacity: 2,000 tiles * 256 KB = ~512 MB ceiling.
pub const DEFAULT_TILE_CACHE_CAPACITY: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub slide_id: String,
    pub level: u32,
    pub tile_x: u32,
    pub tile_y: u32,
}

impl TileKey {
    pub fn new(slide_id: impl Into<String>, level: u32, tile_x: u32, tile_y: u32) -> Self {
        Self {
            slide_id: slide_id.into(),
            level,
            tile_x,
            tile_y,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CachedTile {
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

impl CachedTile {
    pub fn new(rgba: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            rgba: Arc::new(rgba),
            width,
            height,
        }
    }

    pub fn byte_size(&self) -> usize {
        self.rgba.len()
    }
}

/// Thread-safe bounded LRU tile cache.
pub struct TileCache {
    inner: Mutex<LruCache<TileKey, CachedTile>>,
    max_capacity: usize,
}

impl TileCache {
    pub fn new(capacity: usize) -> Self {
        let non_zero_cap = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::new(1).unwrap());
        Self {
            inner: Mutex::new(LruCache::new(non_zero_cap)),
            max_capacity: capacity,
        }
    }

    pub fn with_default_ceiling() -> Self {
        Self::new(DEFAULT_TILE_CACHE_CAPACITY)
    }

    pub fn get(&self, key: &TileKey) -> Option<CachedTile> {
        let mut guard = self.inner.lock().unwrap();
        guard.get(key).cloned()
    }

    pub fn insert(&self, key: TileKey, tile: CachedTile) {
        let mut guard = self.inner.lock().unwrap();
        guard.put(key, tile);
    }

    pub fn len(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn capacity(&self) -> usize {
        self.max_capacity
    }

    pub fn clear(&self) {
        let mut guard = self.inner.lock().unwrap();
        guard.clear();
    }

    /// Approximate total bytes in cache.
    pub fn total_bytes(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.iter().map(|(_, tile)| tile.byte_size()).sum()
    }
}

impl Default for TileCache {
    fn default() -> Self {
        Self::with_default_ceiling()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_cache_basic_operations() {
        let cache = TileCache::new(3);
        let key1 = TileKey::new("slide1", 0, 0, 0);
        let key2 = TileKey::new("slide1", 0, 0, 1);
        let key3 = TileKey::new("slide1", 0, 1, 0);
        let key4 = TileKey::new("slide1", 0, 1, 1);

        let tile = CachedTile::new(vec![255; 256 * 256 * 4], 256, 256);

        cache.insert(key1.clone(), tile.clone());
        cache.insert(key2.clone(), tile.clone());
        cache.insert(key3.clone(), tile.clone());

        assert_eq!(cache.len(), 3);
        assert!(cache.get(&key1).is_some());

        // Inserting a 4th tile should evict the least recently used
        // Since key1 was accessed with get, key2 is now the LRU
        cache.insert(key4.clone(), tile.clone());

        assert_eq!(cache.len(), 3);
        assert!(cache.get(&key1).is_some());
        assert!(cache.get(&key2).is_none(), "Key 2 should be evicted");
        assert!(cache.get(&key3).is_some());
        assert!(cache.get(&key4).is_some());
    }

    #[test]
    fn test_tile_cache_byte_budget_accounting() {
        let cache = TileCache::new(10);
        let tile_bytes = 256 * 256 * 4; // 256 KB
        let tile = CachedTile::new(vec![0; tile_bytes], 256, 256);

        for i in 0..5 {
            cache.insert(TileKey::new("slide_a", 0, i, 0), tile.clone());
        }

        assert_eq!(cache.len(), 5);
        assert_eq!(cache.total_bytes(), 5 * tile_bytes);
    }

    #[test]
    fn test_tile_cache_high_concurrency() {
        let capacity = 50;
        let cache = Arc::new(TileCache::new(capacity));
        let tile = CachedTile::new(vec![42; 256 * 256 * 4], 256, 256);

        let mut handles = Vec::new();
        for thread_id in 0..16 {
            let cache_clone = Arc::clone(&cache);
            let tile_clone = tile.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..100 {
                    let key = TileKey::new("slide_concurrent", 0, thread_id * 100 + i, 0);
                    cache_clone.insert(key.clone(), tile_clone.clone());
                    let _ = cache_clone.get(&key);
                }
            }));
        }

        for h in handles {
            h.join().expect("Worker thread should succeed without panicking");
        }

        // Cache must never exceed bounded capacity
        assert!(cache.len() <= capacity);
    }
}
