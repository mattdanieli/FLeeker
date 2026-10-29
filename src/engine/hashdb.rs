//! Impronte BLAKE3 dei file.
//!
//! - `cache`: per ogni file gia letto, dimensione + data di modifica + impronta.
//!   Se dimensione e data non cambiano, non serve rileggere il file.
//! - `fingerprints`: l'impronta di ogni preferito che funzionava. Se domani il
//!   file viene spostato E rinominato, lo ritroviamo dal contenuto.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::paths::{basename_any, norm_key};

pub type Hash = [u8; 32];

pub fn hex(h: &Hash) -> String {
    blake3::Hash::from_bytes(*h).to_hex().to_string()
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CacheEntry {
    pub size: u64,
    pub mtime: i64,
    #[serde(with = "hash_hex")]
    pub hash: Hash,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fingerprint {
    #[serde(with = "hash_hex")]
    pub hash: Hash,
    pub size: u64,
    pub name: String,
    /// Data dell'ultima volta che il file e stato visto funzionare.
    pub seen: String,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct HashDb {
    pub version: u32,
    #[serde(default)]
    pub cache: HashMap<String, CacheEntry>,
    #[serde(default)]
    pub fingerprints: HashMap<String, Fingerprint>,
    #[serde(skip)]
    pub dirty: bool,
}

mod hash_hex {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(h: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::hex(h))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        blake3::Hash::from_hex(&s).map(|h| *h.as_bytes()).map_err(serde::de::Error::custom)
    }
}

pub fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("FLEEKER_DB") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    let path = base.join(super::APP_ID).join("content-index.json");
    // Riusa l'indice della versione precedente (Ripara Preferiti), se c'e
    let old = base.join("RiparaPreferitiFL").join("impronte.json");
    if !path.exists() && old.exists() {
        let _ = fs::create_dir_all(path.parent().unwrap());
        let _ = fs::copy(&old, &path);
    }
    path
}

/// Dimensione e data di modifica (secondi). None se il file non c'e.
pub fn stat(path: &Path) -> Option<(u64, i64)> {
    let m = fs::metadata(path).ok()?;
    if !m.is_file() {
        return None;
    }
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some((m.len(), mtime))
}

/// Impronta BLAKE3 del contenuto. Per i file grandi usa la mappatura in
/// memoria e piu core insieme.
pub fn hash_file(path: &Path) -> std::io::Result<Hash> {
    let mut h = blake3::Hasher::new();
    let len = fs::metadata(path)?.len();
    if len > 16 * 1024 * 1024 {
        h.update_mmap_rayon(path)?;
    } else {
        h.update_mmap(path)?;
    }
    Ok(*h.finalize().as_bytes())
}

impl HashDb {
    pub fn load(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|d| serde_json::from_slice::<HashDb>(&d).ok())
            .unwrap_or_else(|| HashDb { version: 1, ..Default::default() })
    }

    pub fn save(&mut self, path: &Path) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        fs::rename(&tmp, path)?;
        self.dirty = false;
        Ok(())
    }

    /// Impronta gia nota, se il file non e cambiato.
    pub fn cached(&self, path: &str, size: u64, mtime: i64) -> Option<Hash> {
        self.cache
            .get(&norm_key(path))
            .filter(|c| c.size == size && c.mtime == mtime)
            .map(|c| c.hash)
    }

    pub fn put_cache(&mut self, path: &str, size: u64, mtime: i64, hash: Hash) {
        self.cache.insert(norm_key(path), CacheEntry { size, mtime, hash });
        self.dirty = true;
    }

    pub fn fingerprint(&self, path: &str) -> Option<&Fingerprint> {
        self.fingerprints.get(&norm_key(path))
    }

    pub fn put_fingerprint(&mut self, path: &str, hash: Hash, size: u64) {
        let key = norm_key(path);
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        match self.fingerprints.get_mut(&key) {
            Some(f) if f.hash == hash && f.size == size && f.seen == today => {}
            _ => {
                self.fingerprints.insert(
                    key,
                    Fingerprint { hash, size, name: basename_any(path).to_lowercase(), seen: today },
                );
                self.dirty = true;
            }
        }
    }

    /// Dopo una riparazione, l'impronta segue il file nel nuovo percorso.
    pub fn move_fingerprint(&mut self, old: &str, new: &str) {
        if let Some(f) = self.fingerprints.get(&norm_key(old)).cloned() {
            self.fingerprints.insert(norm_key(new), Fingerprint { name: basename_any(new).to_lowercase(), ..f });
            self.dirty = true;
        }
    }

    /// Togli dalla cache i file che non esistono piu (tiene il file piccolo).
    pub fn prune_cache(&mut self, max: usize) {
        if self.cache.len() <= max {
            return;
        }
        self.cache.retain(|k, _| Path::new(k).exists());
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_e_db() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.wav");
        fs::write(&f, b"ciao").unwrap();
        let h = hash_file(&f).unwrap();
        assert_eq!(h, *blake3::hash(b"ciao").as_bytes());

        let dbp = dir.path().join("db.json");
        let mut db = HashDb::load(&dbp);
        let (s, m) = stat(&f).unwrap();
        db.put_cache(f.to_str().unwrap(), s, m, h);
        db.put_fingerprint(f.to_str().unwrap(), h, s);
        db.save(&dbp).unwrap();
        let db2 = HashDb::load(&dbp);
        assert_eq!(db2.cached(f.to_str().unwrap(), s, m), Some(h));
        assert_eq!(db2.fingerprint(f.to_str().unwrap()).unwrap().hash, h);
    }
}
