//! Ricerca dei file nelle cartelle scelte, in parallelo su tutti i core.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;

/// Contatori letti dall'interfaccia mentre la ricerca lavora.
#[derive(Default)]
pub struct Progress {
    pub dirs: AtomicU64,
    pub files: AtomicU64,
    pub hashed_files: AtomicU64,
    pub hashed_bytes: AtomicU64,
    pub to_hash: AtomicU64,
    pub cancel: AtomicBool,
}

#[derive(Clone, Debug)]
pub struct FileHit {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

/// Cosa cerchiamo: nomi di file (minuscoli) e dimensioni esatte
/// (per ritrovare i file rinominati dalla loro impronta).
pub struct Wanted<'a> {
    pub names: &'a HashSet<String>,
    pub sizes: &'a HashSet<u64>,
}

#[derive(Default)]
pub struct ScanResult {
    /// nome minuscolo -> file trovati con quel nome
    pub by_name: HashMap<String, Vec<FileHit>>,
    /// dimensione -> file trovati con quella dimensione
    pub by_size: HashMap<u64, Vec<FileHit>>,
    pub cancelled: bool,
}

pub fn scan(folders: &[PathBuf], wanted: &Wanted, progress: &Progress) -> ScanResult {
    let by_name = Mutex::new(HashMap::<String, Vec<FileHit>>::new());
    let by_size = Mutex::new(HashMap::<u64, Vec<FileHit>>::new());
    let visited = Mutex::new(HashSet::<String>::new());

    folders.par_iter().for_each(|f| walk(f, wanted, progress, &by_name, &by_size, &visited));

    ScanResult {
        by_name: by_name.into_inner().unwrap(),
        by_size: by_size.into_inner().unwrap(),
        cancelled: progress.cancel.load(Ordering::Relaxed),
    }
}

fn walk(
    dir: &Path,
    wanted: &Wanted,
    progress: &Progress,
    by_name: &Mutex<HashMap<String, Vec<FileHit>>>,
    by_size: &Mutex<HashMap<u64, Vec<FileHit>>>,
    visited: &Mutex<HashSet<String>>,
) {
    if progress.cancel.load(Ordering::Relaxed) {
        return;
    }
    // Se due cartelle scelte si sovrappongono, ogni cartella si visita una volta sola.
    let key = super::paths::norm_key(&dir.to_string_lossy());
    if !visited.lock().unwrap().insert(key) {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    progress.dirs.fetch_add(1, Ordering::Relaxed);

    let mut subdirs = Vec::new();
    let mut n_files = 0u64;
    for d in rd.flatten() {
        let Ok(ft) = d.file_type() else { continue };
        if ft.is_symlink() {
            continue; // niente giri infiniti con collegamenti e junction
        }
        if ft.is_dir() {
            subdirs.push(d.path());
            continue;
        }
        n_files += 1;
        let name = d.file_name().to_string_lossy().to_lowercase();
        let name_match = wanted.names.contains(&name);
        if !name_match && wanted.sizes.is_empty() {
            continue;
        }
        let Ok(meta) = d.metadata() else { continue };
        let size = meta.len();
        let size_match = wanted.sizes.contains(&size);
        if !name_match && !size_match {
            continue;
        }
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let hit = FileHit { path: d.path().to_string_lossy().into_owned(), size, mtime };
        if size_match {
            by_size.lock().unwrap().entry(size).or_default().push(hit.clone());
        }
        if name_match {
            by_name.lock().unwrap().entry(name).or_default().push(hit);
        }
    }
    progress.files.fetch_add(n_files, Ordering::Relaxed);

    subdirs.par_iter().for_each(|s| walk(s, wanted, progress, by_name, by_size, visited));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trova_per_nome_e_dimensione() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("x").join("y");
        fs::create_dir_all(&a).unwrap();
        fs::write(a.join("Kick 01.WAV"), b"12345").unwrap();
        fs::write(a.join("altro.wav"), b"abc").unwrap();
        let names: HashSet<String> = ["kick 01.wav".to_string()].into();
        let sizes: HashSet<u64> = [3].into();
        let p = Progress::default();
        let r = scan(&[dir.path().to_path_buf()], &Wanted { names: &names, sizes: &sizes }, &p);
        assert_eq!(r.by_name["kick 01.wav"].len(), 1);
        assert_eq!(r.by_size[&3].len(), 1);
        assert_eq!(p.dirs.load(Ordering::Relaxed), 3);
    }
}
