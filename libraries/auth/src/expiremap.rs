use std::{borrow::Borrow, collections::HashMap, hash::Hash, iter::FromIterator, sync::Arc};

use tokio::{sync::RwLock, time::Instant};

#[derive(Debug, Default, Clone)]
pub struct ExpireMap<K, V>
where
    K: Eq + Hash + Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    map: Arc<RwLock<HashMap<K, V>>>,
}

impl<K, V> ExpireMap<K, V>
where
    K: Eq + Hash + Send + Sync + Clone + 'static,
    V: Send + Sync + Clone + 'static,
{
    pub async fn insert(&self, key: K, value: V, expiry: Option<Instant>) {
        self.map.write().await.insert(key.clone(), value);

        if let Some(expiry) = expiry {
            let arc_map = Arc::clone(&self.map);
            tokio::task::spawn(async move {
                tokio::time::sleep_until(expiry).await;
                arc_map.write().await.remove(&key);
            });
        }
    }

    pub async fn contains<Q: ?Sized>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.map.read().await.contains_key(key)
    }

    pub async fn remove<Q: ?Sized>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.map.write().await.remove(key)
    }

    pub async fn snapshot(&'_ self) -> tokio::sync::RwLockReadGuard<'_, HashMap<K, V>> {
        self.map.read().await
    }

    pub async fn snapshot_mut(&'_ self) -> tokio::sync::RwLockWriteGuard<'_, HashMap<K, V>> {
        self.map.write().await
    }
}

impl FromIterator<String> for ExpireMap<String, ()> {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        // generate the backing hashmap directly since we need to
        // not be async here and do not care about expiry
        Self {
            map: Arc::new(RwLock::new(iter.into_iter().fold(
                HashMap::new(),
                |mut map, value| {
                    map.insert(value, ());
                    map
                },
            ))),
        }
    }
}
