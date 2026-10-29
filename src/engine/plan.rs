//! Decide cosa fare per ogni preferito.

use std::collections::{HashMap, HashSet};

use super::hashdb::Hash;
use super::paths::{basename_any, drive_of, norm_key, shared_tail_dirs};
use super::tags::Edit;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Keep,
    Update,
    Remove,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Il file esiste.
    Ok,
    /// Il file esiste, ma ha lo stesso contenuto di un altro preferito.
    OkSameContent,
    /// Mancante, la ricerca non e ancora stata fatta.
    Pending,
    /// Stesso percorso di una riga precedente.
    DupPath,
    /// C'e gia un preferito funzionante con lo stesso nome (o lo stesso contenuto).
    AlreadyStarred,
    /// Ritrovato con lo stesso nome.
    Found,
    /// Ritrovato dal contenuto anche se e stato rinominato.
    FoundRenamed,
    /// Ci sono piu file diversi con questo nome: serve un controllo.
    Ambiguous,
    /// Non trovato.
    NotFound,
    /// Il disco del vecchio percorso non e collegato.
    DriveMissing,
}

impl Status {
    pub fn is_missing(self) -> bool {
        !matches!(self, Status::Ok | Status::OkSameContent)
    }
}

/// Perche una riga ha quello stato. Il testo lo sceglie l'interfaccia, nella lingua giusta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    Valid,
    Pending,
    DuplicateEntry,
    SameContentAs(String),
    AlreadyStarredContent(String),
    AlreadyStarredName,
    Verified { extra: usize },
    Renamed { new_name: String, extra: usize },
    Located { copies: usize },
    DifferentContent { n: usize },
    Versions { n: usize },
    NotFound,
    DriveOffline(String),
}

/// Un file trovato che potrebbe essere il preferito mancante.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: String,
    pub size: u64,
    pub hash: Option<Hash>,
    /// Some(true) se il contenuto e identico a quello registrato del preferito.
    pub matches_original: Option<bool>,
}

/// Quello che sappiamo di un preferito dopo il caricamento.
#[derive(Clone, Debug, Default)]
pub struct EntryInfo {
    pub path: String,
    pub exists: bool,
    pub size: Option<u64>,
    pub hash: Option<Hash>,
    /// Impronta registrata quando il file funzionava (hash, dimensione).
    pub fingerprint: Option<(Hash, u64)>,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub action: Action,
    pub status: Status,
    pub reason: Reason,
    pub candidates: Vec<Candidate>,
    pub chosen: usize,
    /// Per i doppioni: indice del preferito "originale".
    pub same_as: Option<usize>,
}

impl Row {
    fn new(status: Status, action: Action, reason: Reason) -> Self {
        Row { action, status, reason, candidates: vec![], chosen: 0, same_as: None }
    }
    pub fn new_path(&self) -> Option<&str> {
        (self.action == Action::Update).then(|| self.candidates.get(self.chosen).map(|c| c.path.as_str())).flatten()
    }
}

/// Stato iniziale, prima della ricerca.
pub fn initial_rows(infos: &[EntryInfo]) -> Vec<Row> {
    let mut seen = HashSet::new();
    let mut by_hash: HashMap<Hash, usize> = HashMap::new();
    infos
        .iter()
        .enumerate()
        .map(|(i, e)| {
            if !seen.insert(norm_key(&e.path)) {
                return Row::new(Status::DupPath, Action::Remove, Reason::DuplicateEntry);
            }
            if e.exists {
                if let Some(h) = e.hash {
                    if let Some(&first) = by_hash.get(&h) {
                        let mut r = Row::new(
                            Status::OkSameContent,
                            Action::Keep,
                            Reason::SameContentAs(basename_any(&infos[first].path).to_string()),
                        );
                        r.same_as = Some(first);
                        return r;
                    }
                    by_hash.insert(h, i);
                }
                return Row::new(Status::Ok, Action::Keep, Reason::Valid);
            }
            Row::new(Status::Pending, Action::Keep, Reason::Pending)
        })
        .collect()
}

/// Risultati della ricerca, gia con le impronte calcolate.
pub struct Found<'a> {
    pub by_name: &'a HashMap<String, Vec<Candidate>>,
    pub by_size: &'a HashMap<u64, Vec<Candidate>>,
}

/// Ordine di preferenza: stesse cartelle finali del vecchio percorso, poi percorso piu corto.
fn rank(old: &str, cands: &mut [Candidate]) {
    cands.sort_by(|a, b| {
        shared_tail_dirs(old, &b.path)
            .cmp(&shared_tail_dirs(old, &a.path))
            .then(a.path.len().cmp(&b.path.len()))
            .then(a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
}

pub fn plan(infos: &[EntryInfo], found: &Found, drive_exists: &dyn Fn(&str) -> bool) -> Vec<Row> {
    let mut rows = initial_rows(infos);

    // nome -> impronte dei preferiti che funzionano con quel nome
    let mut working_names: HashMap<String, Vec<Option<Hash>>> = HashMap::new();
    let mut working_hashes: HashMap<Hash, usize> = HashMap::new();
    let mut taken: HashSet<String> = HashSet::new();
    for (i, e) in infos.iter().enumerate() {
        if e.exists {
            working_names.entry(basename_any(&e.path).to_lowercase()).or_default().push(e.hash);
            if let Some(h) = e.hash {
                working_hashes.entry(h).or_insert(i);
            }
            taken.insert(norm_key(&e.path));
        }
    }

    for (i, e) in infos.iter().enumerate() {
        if rows[i].status != Status::Pending {
            continue;
        }
        let name = basename_any(&e.path).to_lowercase();
        let fp = e.fingerprint;

        // 1) Lo stesso contenuto e gia tra i preferiti (magari con un altro nome)
        if let Some((h, _)) = fp {
            if let Some(&w) = working_hashes.get(&h) {
                let mut r = Row::new(
                    Status::AlreadyStarred,
                    Action::Remove,
                    Reason::AlreadyStarredContent(basename_any(&infos[w].path).to_string()),
                );
                r.same_as = Some(w);
                rows[i] = r;
                continue;
            }
        }

        // 2) C'e gia un preferito funzionante con lo stesso nome
        if let Some(hashes) = working_names.get(&name) {
            // Se conosciamo l'impronta e quello e un file diverso, non e un doppione
            let different = fp.is_some_and(|(h, _)| hashes.iter().all(|wh| wh.is_some_and(|wh| wh != h)));
            if !different {
                rows[i] = Row::new(
                    Status::AlreadyStarred,
                    Action::Remove,
                    Reason::AlreadyStarredName,
                );
                continue;
            }
        }

        let free = |c: &&Candidate| !taken.contains(&norm_key(&c.path));
        let mut by_name: Vec<Candidate> =
            found.by_name.get(&name).map(|v| v.iter().filter(free).cloned().collect()).unwrap_or_default();

        let mut row = if let Some((fh, fsize)) = fp {
            for c in &mut by_name {
                c.matches_original = c.hash.map(|h| h == fh);
            }
            let mut exact: Vec<Candidate> = by_name.iter().filter(|c| c.matches_original == Some(true)).cloned().collect();
            let renamed: Vec<Candidate> = found
                .by_size
                .get(&fsize)
                .map(|v| {
                    v.iter()
                        .filter(free)
                        .filter(|c| c.hash == Some(fh))
                        .filter(|c| !exact.iter().any(|x| norm_key(&x.path) == norm_key(&c.path)))
                        .map(|c| Candidate { matches_original: Some(true), ..c.clone() })
                        .collect()
                })
                .unwrap_or_default();
            let same_name_exact = !exact.is_empty();
            exact.extend(renamed);
            if !exact.is_empty() {
                rank(&e.path, &mut exact);
                // a parita, meglio un file con lo stesso nome
                if same_name_exact {
                    exact.sort_by_key(|c| basename_any(&c.path).to_lowercase() != name);
                }
                let renamed_pick = basename_any(&exact[0].path).to_lowercase() != name;
                let extra = exact.len() - 1;
                let reason = if renamed_pick {
                    Reason::Renamed { new_name: basename_any(&exact[0].path).to_string(), extra }
                } else {
                    Reason::Verified { extra }
                };
                let mut r = Row::new(
                    if renamed_pick { Status::FoundRenamed } else { Status::Found },
                    Action::Update,
                    reason,
                );
                r.candidates = exact;
                r
            } else if !by_name.is_empty() {
                rank(&e.path, &mut by_name);
                let n = by_name.len();
                let mut r = Row::new(
                    Status::Ambiguous,
                    Action::Keep,
                    Reason::DifferentContent { n },
                );
                r.candidates = by_name;
                r
            } else {
                not_found(&e.path, drive_exists)
            }
        } else if by_name.is_empty() {
            not_found(&e.path, drive_exists)
        } else {
            rank(&e.path, &mut by_name);
            let versions: HashSet<Option<Hash>> = by_name.iter().map(|c| c.hash).collect();
            let n = by_name.len();
            if versions.len() == 1 {
                let mut r = Row::new(Status::Found, Action::Update, Reason::Located { copies: n });
                r.candidates = by_name;
                r
            } else {
                let mut r = Row::new(
                    Status::Ambiguous,
                    Action::Update,
                    Reason::Versions { n: versions.len() },
                );
                r.candidates = by_name;
                r
            }
        };

        if row.action == Action::Update {
            if let Some(c) = row.candidates.get(row.chosen) {
                taken.insert(norm_key(&c.path));
                working_names.entry(name.clone()).or_default().push(c.hash);
            }
        }
        row.chosen = 0;
        rows[i] = row;
    }
    rows
}

fn not_found(path: &str, drive_exists: &dyn Fn(&str) -> bool) -> Row {
    match drive_of(path) {
        Some(d) if !drive_exists(&d) => Row::new(
            Status::DriveMissing,
            Action::Keep,
            Reason::DriveOffline(d[..2].to_uppercase()),
        ),
        _ => Row::new(Status::NotFound, Action::Keep, Reason::NotFound),
    }
}

/// Trasforma le righe nelle modifiche da scrivere.
pub fn edits(rows: &[Row]) -> Vec<Edit> {
    rows.iter()
        .map(|r| match r.action {
            Action::Keep => Edit::Keep,
            Action::Remove => Edit::Remove,
            Action::Update => match r.new_path() {
                Some(p) => Edit::Replace(p.to_string()),
                None => Edit::Keep,
            },
        })
        .collect()
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Summary {
    pub total: usize,
    pub working: usize,
    pub missing: usize,
    pub update: usize,
    pub remove: usize,
    pub check: usize,
    pub same_content: usize,
}

pub fn summary(rows: &[Row]) -> Summary {
    let mut s = Summary { total: rows.len(), ..Default::default() };
    for r in rows {
        if r.status.is_missing() && r.status != Status::DupPath {
            s.missing += 1;
        } else if !r.status.is_missing() {
            s.working += 1;
        }
        match r.action {
            Action::Update => s.update += 1,
            Action::Remove => s.remove += 1,
            Action::Keep => {}
        }
        if r.status == Status::Ambiguous {
            s.check += 1;
        }
        if r.status == Status::OkSameContent {
            s.same_content += 1;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u8) -> Hash {
        [n; 32]
    }
    fn info(path: &str, exists: bool, hash: Option<Hash>, fp: Option<(Hash, u64)>) -> EntryInfo {
        EntryInfo { path: path.into(), exists, size: None, hash, fingerprint: fp }
    }
    fn cand(path: &str, size: u64, hash: u8) -> Candidate {
        Candidate { path: path.into(), size, hash: Some(h(hash)), matches_original: None }
    }

    #[test]
    fn casi_principali() {
        let infos = vec![
            info("c:\\old\\kick.wav", true, Some(h(1)), None),         // 0 funziona
            info("c:\\old\\kick.wav", false, None, None),              // 1 ripetuto
            info("c:\\gone\\kick.wav", false, None, None),             // 2 stesso nome di uno che funziona
            info("c:\\gone\\snare.wav", false, None, None),            // 3 trovato
            info("c:\\gone\\pack\\hat.wav", false, None, None),        // 4 ambiguo, sceglie "pack"
            info("c:\\gone\\lead.wav", false, None, Some((h(9), 50))), // 5 rinominato
            info("c:\\gone\\nope.wav", false, None, None),             // 6 non trovato
            info("z:\\x\\nope2.wav", false, None, None),               // 7 disco scollegato
            info("c:\\other\\kick2.wav", true, Some(h(1)), None),      // 8 doppione di contenuto
        ];
        let by_name: HashMap<String, Vec<Candidate>> = [
            ("snare.wav".to_string(), vec![cand("d:\\s\\snare.wav", 1, 2)]),
            ("hat.wav".to_string(), vec![cand("d:\\aaa\\hat.wav", 1, 3), cand("d:\\libs\\pack\\hat.wav", 1, 4)]),
        ]
        .into();
        let by_size: HashMap<u64, Vec<Candidate>> = [(50, vec![cand("d:\\new\\Lead Final.wav", 50, 9)])].into();
        let rows = plan(&infos, &Found { by_name: &by_name, by_size: &by_size }, &|d| d != "z:\\");

        assert_eq!(rows[0].status, Status::Ok);
        assert_eq!((rows[1].status, rows[1].action), (Status::DupPath, Action::Remove));
        assert_eq!((rows[2].status, rows[2].action), (Status::AlreadyStarred, Action::Remove));
        assert_eq!(rows[3].new_path(), Some("d:\\s\\snare.wav"));
        assert_eq!(rows[4].status, Status::Ambiguous);
        assert_eq!(rows[4].new_path(), Some("d:\\libs\\pack\\hat.wav"));
        assert_eq!(rows[5].status, Status::FoundRenamed);
        assert_eq!(rows[5].new_path(), Some("d:\\new\\Lead Final.wav"));
        assert_eq!(rows[6].status, Status::NotFound);
        assert_eq!(rows[7].status, Status::DriveMissing);
        assert_eq!(rows[8].status, Status::OkSameContent);
        assert_eq!(rows[8].same_as, Some(0));
    }

    #[test]
    fn stesso_nome_ma_file_diverso_non_si_cancella() {
        // "kick 01.wav" di due pack diversi: con l'impronta sappiamo che non sono lo stesso file
        let infos = vec![
            info("c:\\packA\\kick 01.wav", true, Some(h(1)), None),
            info("c:\\packB\\kick 01.wav", false, None, Some((h(2), 10))),
        ];
        let by_name: HashMap<String, Vec<Candidate>> =
            [("kick 01.wav".to_string(), vec![cand("d:\\moved\\packB\\kick 01.wav", 10, 2)])].into();
        let by_size = HashMap::new();
        let rows = plan(&infos, &Found { by_name: &by_name, by_size: &by_size }, &|_| true);
        assert_eq!(rows[1].status, Status::Found);
        assert_eq!(rows[1].new_path(), Some("d:\\moved\\packB\\kick 01.wav"));
    }

    #[test]
    fn copie_identiche_non_sono_ambigue() {
        let infos = vec![info("c:\\gone\\clap.wav", false, None, None)];
        let by_name: HashMap<String, Vec<Candidate>> =
            [("clap.wav".to_string(), vec![cand("d:\\a\\clap.wav", 1, 5), cand("e:\\b\\clap.wav", 1, 5)])].into();
        let by_size = HashMap::new();
        let rows = plan(&infos, &Found { by_name: &by_name, by_size: &by_size }, &|_| true);
        assert_eq!(rows[0].status, Status::Found);
        assert_eq!(rows[0].candidates.len(), 2);
    }

    #[test]
    fn edits_allineati() {
        let infos = vec![info("c:\\a.wav", true, None, None), info("c:\\a.wav", false, None, None)];
        let rows = initial_rows(&infos);
        assert_eq!(edits(&rows), vec![Edit::Keep, Edit::Remove]);
    }
}
