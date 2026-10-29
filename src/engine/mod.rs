//! Motore: carica il file Tags, cerca i file mancanti, calcola le impronte.

/// Nome usato per la cartella dei dati e le impostazioni.
pub const APP_ID: &str = "FLeeker";

pub mod hashdb;
pub mod paths;
pub mod plan;
pub mod scan;
pub mod system;
pub mod tags;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use rayon::prelude::*;

use hashdb::{Hash, HashDb};
use paths::{basename_any, norm_key, parent_any};
use plan::{Candidate, EntryInfo, Row};
use scan::{FileHit, Progress, Wanted};
use tags::TagsFile;

pub struct Loaded {
    pub tags: TagsFile,
    pub infos: Vec<EntryInfo>,
    pub rows: Vec<Row>,
}

/// Impronta di un file, riusando la cache se il file non e cambiato.
fn hash_cached(db: &Mutex<HashDb>, path: &str, size: u64, mtime: i64, progress: &Progress) -> Option<Hash> {
    if let Some(h) = db.lock().unwrap().cached(path, size, mtime) {
        progress.hashed_files.fetch_add(1, Ordering::Relaxed);
        return Some(h);
    }
    let h = hashdb::hash_file(Path::new(path)).ok()?;
    progress.hashed_files.fetch_add(1, Ordering::Relaxed);
    progress.hashed_bytes.fetch_add(size, Ordering::Relaxed);
    db.lock().unwrap().put_cache(path, size, mtime, h);
    Some(h)
}

/// Legge il file Tags, controlla quali preferiti esistono e registra
/// l'impronta di quelli che funzionano.
pub fn load(path: &Path, db: &Mutex<HashDb>, progress: &Progress, fingerprints: bool) -> std::io::Result<Loaded> {
    let tags = TagsFile::read(path)?;
    progress.to_hash.store(tags.entries.len() as u64, Ordering::Relaxed);

    let infos: Vec<EntryInfo> = tags
        .entries
        .par_iter()
        .map(|e| {
            let st = hashdb::stat(Path::new(&e.path));
            let hash = match (st, fingerprints) {
                (Some((size, mtime)), true) => hash_cached(db, &e.path, size, mtime, progress),
                _ => {
                    progress.hashed_files.fetch_add(1, Ordering::Relaxed);
                    None
                }
            };
            let fingerprint = db.lock().unwrap().fingerprint(&e.path).map(|f| (f.hash, f.size));
            EntryInfo { path: e.path.clone(), exists: st.is_some(), size: st.map(|s| s.0), hash, fingerprint }
        })
        .collect();

    {
        let mut db = db.lock().unwrap();
        for i in &infos {
            if let (true, Some(h), Some(s)) = (i.exists, i.hash, i.size) {
                db.put_fingerprint(&i.path, h, s);
            }
        }
        db.prune_cache(200_000);
        let _ = db.save(&hashdb::db_path());
    }

    let rows = plan::initial_rows(&infos);
    Ok(Loaded { tags, infos, rows })
}

/// Cartelle suggerite per la ricerca: quelle dei preferiti che esistono ancora,
/// senza ripetere le sottocartelle di cartelle gia in lista.
pub fn suggest_folders(infos: &[EntryInfo]) -> Vec<PathBuf> {
    let mut dirs: Vec<String> = infos
        .iter()
        .map(|i| parent_any(&i.path).to_string())
        .filter(|p| !p.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|p| Path::new(p).is_dir())
        .collect();
    dirs.sort_by_key(|d| norm_key(d).len());
    let mut out: Vec<String> = Vec::new();
    for d in dirs {
        let k = norm_key(&d);
        if !out.iter().any(|o| k.starts_with(&(norm_key(o) + "\\"))) {
            out.push(d);
        }
    }
    out.sort_by_key(|d| norm_key(d));
    out.into_iter().take(8).map(PathBuf::from).collect()
}

pub struct SearchOutput {
    pub rows: Vec<Row>,
    pub cancelled: bool,
    pub dirs: u64,
    pub files: u64,
    pub elapsed: Duration,
}

pub fn search(
    infos: &[EntryInfo],
    folders: &[PathBuf],
    db: &Mutex<HashDb>,
    progress: &Progress,
    use_hash: bool,
) -> SearchOutput {
    let t0 = Instant::now();
    let missing: Vec<&EntryInfo> = infos.iter().filter(|i| !i.exists).collect();
    let names: HashSet<String> = missing.iter().map(|i| basename_any(&i.path).to_lowercase()).collect();
    let sizes: HashSet<u64> = if use_hash {
        missing.iter().filter_map(|i| i.fingerprint.map(|f| f.1)).collect()
    } else {
        HashSet::new()
    };

    let res = scan::scan(folders, &Wanted { names: &names, sizes: &sizes }, progress);

    // Impronte di tutti i file trovati (una volta sola per file)
    let mut unique: HashMap<String, FileHit> = HashMap::new();
    for h in res.by_name.values().chain(res.by_size.values()).flatten() {
        unique.entry(norm_key(&h.path)).or_insert_with(|| h.clone());
    }
    progress.hashed_files.store(0, Ordering::Relaxed);
    progress.to_hash.store(if use_hash { unique.len() as u64 } else { 0 }, Ordering::Relaxed);
    let hashes: HashMap<String, Hash> = if use_hash && !res.cancelled {
        unique
            .par_iter()
            .filter(|_| !progress.cancel.load(Ordering::Relaxed))
            .filter_map(|(k, h)| hash_cached(db, &h.path, h.size, h.mtime, progress).map(|x| (k.clone(), x)))
            .collect()
    } else {
        HashMap::new()
    };
    let _ = db.lock().unwrap().save(&hashdb::db_path());

    let to_cand = |h: &FileHit| Candidate {
        path: h.path.clone(),
        size: h.size,
        hash: hashes.get(&norm_key(&h.path)).copied(),
        matches_original: None,
    };
    let by_name: HashMap<String, Vec<Candidate>> =
        res.by_name.iter().map(|(k, v)| (k.clone(), v.iter().map(to_cand).collect())).collect();
    let by_size: HashMap<u64, Vec<Candidate>> =
        res.by_size.iter().map(|(k, v)| (*k, v.iter().map(to_cand).collect())).collect();

    let rows = plan::plan(infos, &plan::Found { by_name: &by_name, by_size: &by_size }, &system::drive_exists);

    SearchOutput {
        rows,
        cancelled: res.cancelled || progress.cancel.load(Ordering::Relaxed),
        dirs: progress.dirs.load(Ordering::Relaxed),
        files: progress.files.load(Ordering::Relaxed),
        elapsed: t0.elapsed(),
    }
}

/// Scrive il file Tags (con backup) e sposta le impronte sui nuovi percorsi.
pub fn apply(tags: &TagsFile, infos: &[EntryInfo], rows: &[Row], db: &Mutex<HashDb>) -> std::io::Result<PathBuf> {
    let data = tags.build(&plan::edits(rows));
    let backup = tags::write_atomic(&tags.path, &data)?;
    let mut db = db.lock().unwrap();
    for (info, row) in infos.iter().zip(rows) {
        if let Some(new) = row.new_path() {
            db.move_fingerprint(&info.path, new);
            if let Some(h) = row.candidates.get(row.chosen).and_then(|c| c.hash) {
                db.put_fingerprint(new, h, row.candidates[row.chosen].size);
            }
        }
    }
    let _ = db.save(&hashdb::db_path());
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plan::{Action, Status};
    use std::fs;

    /// Percorso completo: carico, sposto e rinomino i file, ricarico, cerco, applico.
    #[test]
    fn flusso_completo() {
        // Prefisso con maiuscole: con un Tags tutto minuscolo i nuovi percorsi verrebbero scritti in minuscolo
        let dir = tempfile::Builder::new().prefix("FLeeker").tempdir().unwrap();
        let root = dir.path();
        unsafe { std::env::set_var("FLEEKER_DB", root.join("db.json")) };
        let old = root.join("old pack");
        let new = root.join("libreria").join("old pack");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::create_dir_all(root.join("altro")).unwrap();
        let files = [("kick 01.wav", "KICK"), ("snare.wav", "SNARE"), ("lead.wav", "LEAD-UNICO"), ("pad.wav", "PAD")];
        for (n, c) in files {
            fs::write(old.join(n), c).unwrap();
        }
        let line = |n: &str| format!("\"{}\",*", old.join(n).display());
        let tags = root.join("Tags");
        let body = format!("@TagCase=vst3,*,plugin,vst\r\n{}\r\n{}\r\n{}\r\n{}\r\n", line("kick 01.wav"), line("snare.wav"), line("lead.wav"), line("pad.wav"));
        fs::write(&tags, &body).unwrap();

        let db = Mutex::new(HashDb::load(&hashdb::db_path()));
        let pr = Progress::default();
        let l = load(&tags, &db, &pr, true).unwrap();
        assert!(l.infos.iter().all(|i| i.exists));
        assert_eq!(db.lock().unwrap().fingerprints.len(), 4);

        // Sposto tutto; "lead.wav" lo rinomino; "snare.wav" ha anche una copia diversa altrove
        fs::rename(old.join("kick 01.wav"), new.join("kick 01.wav")).unwrap();
        fs::rename(old.join("snare.wav"), new.join("snare.wav")).unwrap();
        fs::rename(old.join("lead.wav"), root.join("altro").join("Lead FINAL v2.wav")).unwrap();
        fs::write(root.join("altro").join("snare.wav"), "SNARE-DIVERSO").unwrap();
        fs::remove_file(old.join("pad.wav")).unwrap();

        let l = load(&tags, &db, &pr, true).unwrap();
        assert_eq!(l.infos.iter().filter(|i| !i.exists).count(), 4);
        let out = search(&l.infos, &[root.join("libreria"), root.join("altro")], &db, &Progress::default(), true);
        let r = &out.rows;
        assert_eq!(r[0].status, Status::Found);
        assert_eq!(r[1].status, Status::Found, "{:?}", r[1].reason);
        assert!(r[1].new_path().unwrap().contains("libreria"));
        assert_eq!(r[2].status, Status::FoundRenamed, "{:?}", r[2].reason);
        assert!(r[2].new_path().unwrap().ends_with("Lead FINAL v2.wav"));
        assert_eq!((r[3].status, r[3].action), (Status::NotFound, Action::Keep));

        let backup = apply(&l.tags, &l.infos, r, &db).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), body.as_bytes());
        let l2 = load(&tags, &db, &pr, true).unwrap();
        assert_eq!(l2.infos.iter().filter(|i| i.exists).count(), 3);
        let text = fs::read_to_string(&tags).unwrap();
        assert!(text.starts_with("@TagCase=vst3,*,plugin,vst\r\n"));
        assert!(text.ends_with("\r\n"));
    }
}
