//! Interfaccia grafica (inglese, cinese, tedesco, spagnolo, francese, italiano).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Frame, Key, Layout, Margin, RichText, Sense, Stroke,
    TextStyle, Vec2,
};
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use crate::engine::hashdb::{self, HashDb};
use crate::engine::paths::{basename_any, norm_key, parent_any};
use crate::engine::plan::{self, Action, EntryInfo, Reason, Row, Status, Summary};
use crate::engine::scan::Progress;
use crate::engine::system;
use crate::i18n;
use crate::engine::tags::{self, BackupInfo, TagsFile};
use crate::engine::{self, Loaded, SearchOutput};

pub const APP_NAME: &str = "FLeeker";
pub const APP_SUBTITLE: &str = "Sample Seeker for FL Studio";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

// --------------------------------------------------------------------------
// Lingua
// --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lang {
    #[default]
    En,
    Zh,
    De,
    Es,
    Fr,
    It,
}

impl Lang {
    /// Ordine del menu: l'inglese (lingua di partenza) e poi le altre.
    const ALL: [Lang; 6] = [Lang::En, Lang::Zh, Lang::De, Lang::Es, Lang::Fr, Lang::It];

    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Zh => "中文",
            Lang::De => "Deutsch",
            Lang::Es => "Español",
            Lang::Fr => "Français",
            Lang::It => "Italiano",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Lang::En => "EN",
            Lang::Zh => "中文",
            Lang::De => "DE",
            Lang::Es => "ES",
            Lang::Fr => "FR",
            Lang::It => "IT",
        }
    }

    /// Sceglie il testo nella lingua attiva. Inglese e italiano sono scritti nel codice,
    /// le altre lingue stanno in i18n.rs (con l'inglese come chiave e come riserva).
    fn t(self, en: &'static str, it: &'static str) -> &'static str {
        match self {
            Lang::En => en,
            Lang::It => it,
            _ => i18n::get(self, en).unwrap_or(en),
        }
    }

    /// Come `t`, poi sostituisce {0}, {1}, ... con i valori.
    fn f(self, en: &'static str, it: &'static str, args: &[&dyn std::fmt::Display]) -> String {
        let mut s = self.t(en, it).to_string();
        for (i, a) in args.iter().enumerate() {
            s = s.replace(&format!("{{{i}}}"), &a.to_string());
        }
        s
    }
}

fn status_label(l: Lang, s: Status) -> &'static str {
    match s {
        Status::Ok => l.t("Valid", "Valido"),
        Status::OkSameContent => l.t("Duplicate content", "Contenuto duplicato"),
        Status::Pending => l.t("Missing", "Mancante"),
        Status::DupPath => l.t("Duplicate entry", "Voce duplicata"),
        Status::AlreadyStarred => l.t("Already listed", "Già presente"),
        Status::Found => l.t("Located", "Individuato"),
        Status::FoundRenamed => l.t("Renamed", "Rinominato"),
        Status::Ambiguous => l.t("Needs review", "Da verificare"),
        Status::NotFound => l.t("Not found", "Non trovato"),
        Status::DriveMissing => l.t("Drive offline", "Unità scollegata"),
    }
}

fn action_label(l: Lang, a: Action) -> &'static str {
    match a {
        Action::Keep => l.t("Keep", "Mantieni"),
        Action::Update => l.t("Relink", "Ricollega"),
        Action::Remove => l.t("Remove", "Rimuovi"),
    }
}

fn reason_text(l: Lang, r: &Reason) -> String {
    let plus = |extra: usize| match extra {
        0 => String::new(),
        1 => l.f(" (+{0} identical copy)", " (+{0} copia identica)", &[&1]),
        n => l.f(" (+{0} identical copies)", " (+{0} copie identiche)", &[&n]),
    };
    match (r, l) {
        (Reason::Valid, _) => l.t("File is available", "File disponibile").into(),
        (Reason::Pending, _) => l.t("Missing. Run a scan to locate it", "Mancante. Avvia la scansione per individuarlo").into(),
        (Reason::DuplicateEntry, _) => {
            l.t("This entry appears twice in the favorites file", "Questa voce compare due volte nel file dei preferiti").into()
        }
        (Reason::SameContentAs(n), _) => l.f("Identical content to \"{0}\"", "Contenuto identico a \"{0}\"", &[n]),
        (Reason::AlreadyStarredContent(n), _) => l.f(
            "The same file is already a favorite as \"{0}\"",
            "Lo stesso file è già nei preferiti come \"{0}\"",
            &[n],
        ),
        (Reason::AlreadyStarredName, _) => l
            .t("A valid favorite with the same name already exists", "Esiste già un preferito valido con lo stesso nome")
            .into(),
        (Reason::Verified { extra }, _) => {
            l.t("Located, content verified", "Individuato, contenuto verificato").to_string() + &plus(*extra)
        }
        (Reason::Renamed { new_name, extra }, _) => {
            l.f(
                "Located by content, renamed to \"{0}\"",
                "Individuato dal contenuto, rinominato in \"{0}\"",
                &[new_name],
            ) + &plus(*extra)
        }
        (Reason::Located { copies }, _) if *copies > 1 => {
            l.f("Located ({0} identical copies)", "Individuato ({0} copie identiche)", &[copies])
        }
        (Reason::Located { .. }, _) => l.t("Located by file name", "Individuato per nome file").into(),
        (Reason::DifferentContent { n: 1 }, _) => l
            .t(
                "1 file with this name, but the content differs from the original. Select it manually if appropriate",
                "1 file con questo nome, ma il contenuto è diverso dall'originale. Selezionalo manualmente se opportuno",
            )
            .into(),
        (Reason::DifferentContent { n }, _) => l.f(
            "{0} files with this name, but the content differs from the original. Select one manually if appropriate",
            "{0} file con questo nome, ma il contenuto è diverso dall'originale. Selezionane uno manualmente se opportuno",
            &[n],
        ),
        (Reason::Versions { n }, _) => l.f(
            "{0} different versions with this name. The most likely match is selected, please review",
            "{0} versioni diverse con questo nome. È selezionata la più probabile, verifica",
            &[n],
        ),
        (Reason::NotFound, _) => l.t("Not found in the scan locations", "Non trovato nelle posizioni di scansione").into(),
        (Reason::DriveOffline(d), _) => {
            l.f("Drive {0} is not connected. Entry kept", "L'unità {0} non è collegata. Voce mantenuta", &[d])
        }
    }
}

// --------------------------------------------------------------------------
// Colori
// --------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    panel: Color32,
    card: Color32,
    card_hi: Color32,
    border: Color32,
    text: Color32,
    weak: Color32,
    accent: Color32,
    on_accent: Color32,
    green: Color32,
    red: Color32,
    yellow: Color32,
    orange: Color32,
    blue: Color32,
    purple: Color32,
    gray: Color32,
}

const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x14, 0x16, 0x1b),
    panel: Color32::from_rgb(0x1a, 0x1d, 0x23),
    card: Color32::from_rgb(0x1f, 0x23, 0x2b),
    card_hi: Color32::from_rgb(0x2a, 0x2f, 0x39),
    border: Color32::from_rgb(0x30, 0x35, 0x40),
    text: Color32::from_rgb(0xe8, 0xea, 0xf0),
    weak: Color32::from_rgb(0x98, 0x9f, 0xb0),
    accent: Color32::from_rgb(0xff, 0x8a, 0x2b),
    on_accent: Color32::from_rgb(0x1a, 0x10, 0x05),
    green: Color32::from_rgb(0x3e, 0xcf, 0x8e),
    red: Color32::from_rgb(0xff, 0x5f, 0x6d),
    yellow: Color32::from_rgb(0xf2, 0xc1, 0x4e),
    orange: Color32::from_rgb(0xff, 0x9f, 0x43),
    blue: Color32::from_rgb(0x5a, 0xaa, 0xff),
    purple: Color32::from_rgb(0xb5, 0x8c, 0xff),
    gray: Color32::from_rgb(0x80, 0x87, 0x96),
};

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xf3, 0xf4, 0xf7),
    panel: Color32::from_rgb(0xfa, 0xfa, 0xfc),
    card: Color32::from_rgb(0xff, 0xff, 0xff),
    card_hi: Color32::from_rgb(0xee, 0xf0, 0xf5),
    border: Color32::from_rgb(0xdd, 0xe1, 0xe8),
    text: Color32::from_rgb(0x1c, 0x1f, 0x26),
    weak: Color32::from_rgb(0x5c, 0x63, 0x73),
    accent: Color32::from_rgb(0xe8, 0x6e, 0x12),
    on_accent: Color32::WHITE,
    green: Color32::from_rgb(0x12, 0x9a, 0x5c),
    red: Color32::from_rgb(0xd6, 0x33, 0x45),
    yellow: Color32::from_rgb(0xb0, 0x7c, 0x00),
    orange: Color32::from_rgb(0xd9, 0x6a, 0x00),
    blue: Color32::from_rgb(0x1f, 0x6f, 0xd6),
    purple: Color32::from_rgb(0x7a, 0x4c, 0xd9),
    gray: Color32::from_rgb(0x77, 0x7e, 0x8c),
};

fn status_color(p: &Palette, s: Status) -> Color32 {
    match s {
        Status::Ok => p.gray,
        Status::OkSameContent => p.purple,
        Status::Pending | Status::NotFound => p.yellow,
        Status::DupPath | Status::AlreadyStarred => p.red,
        Status::Found => p.green,
        Status::FoundRenamed => p.blue,
        Status::Ambiguous => p.orange,
        Status::DriveMissing => p.gray,
    }
}

fn action_color(p: &Palette, a: Action) -> Color32 {
    match a {
        Action::Keep => p.gray,
        Action::Update => p.green,
        Action::Remove => p.red,
    }
}

fn tint(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}

// --------------------------------------------------------------------------
// Impostazioni salvate tra un avvio e l'altro
// --------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    tags_path: String,
    folders: Vec<PathBuf>,
    use_hash: bool,
    dark: bool,
    /// Lingua scelta dall'utente (si parte sempre in inglese). Il vecchio campo `lang`
    /// non viene piu letto, cosi anche chi aveva l'italiano automatico riparte in inglese.
    language: Lang,
    /// Si riparte sempre in anteprima, per sicurezza.
    #[serde(skip)]
    test_mode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { tags_path: String::new(), folders: vec![], use_hash: true, dark: true, language: Lang::En, test_mode: true }
    }
}

// --------------------------------------------------------------------------
// Stato
// --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    ToFix,
    All,
    Update,
    Remove,
    Check,
    NotFound,
    SameContent,
}

impl Filter {
    const ALL: [Filter; 7] =
        [Filter::ToFix, Filter::All, Filter::Update, Filter::Remove, Filter::Check, Filter::NotFound, Filter::SameContent];

    fn label(self, l: Lang) -> &'static str {
        match self {
            Filter::ToFix => l.t("Action needed", "Da sistemare"),
            Filter::All => l.t("All", "Tutti"),
            Filter::Update => l.t("Relink", "Ricollega"),
            Filter::Remove => l.t("Remove", "Rimuovi"),
            Filter::Check => l.t("Needs review", "Da verificare"),
            Filter::NotFound => l.t("Not found", "Non trovati"),
            Filter::SameContent => l.t("Duplicates", "Duplicati"),
        }
    }
    fn accepts(self, r: &Row) -> bool {
        match self {
            Filter::ToFix => r.status != Status::Ok || r.action != Action::Keep,
            Filter::All => true,
            Filter::Update => r.action == Action::Update,
            Filter::Remove => r.action == Action::Remove,
            Filter::Check => r.status == Status::Ambiguous,
            Filter::NotFound => matches!(r.status, Status::NotFound | Status::DriveMissing | Status::Pending),
            Filter::SameContent => r.status == Status::OkSameContent,
        }
    }
}

enum JobResult {
    Loaded(std::io::Result<Loaded>),
    Searched(SearchOutput),
}

struct Job {
    kind: JobKind,
    progress: Arc<Progress>,
    rx: Receiver<JobResult>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Load,
    Search,
}

enum Dialog {
    None,
    Info { title: String, body: String, good: bool },
    ConfirmApply(Summary),
    Simulation(Summary),
    Backups(Vec<BackupInfo>),
    ConfirmRestore(BackupInfo),
    FlOpen,
}

struct Doc {
    tags: TagsFile,
    infos: Vec<EntryInfo>,
    rows: Vec<Row>,
    /// Le scelte automatiche, per poterle ripristinare.
    auto_rows: Vec<Row>,
    searched: bool,
}

pub struct App {
    settings: Settings,
    lang: Lang,
    pal: Palette,
    db: Arc<Mutex<HashDb>>,
    doc: Option<Doc>,
    tags_candidates: Vec<PathBuf>,
    job: Option<Job>,
    last_search: Option<(u64, u64, Duration, bool)>,
    filter: Filter,
    query: String,
    selected: Option<usize>,
    scroll_to: Option<usize>,
    dialog: Dialog,
    fl_running: Arc<AtomicBool>,
    load_error: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings: Settings = cc.storage.and_then(|s| eframe::get_value(s, eframe::APP_KEY)).unwrap_or_default();
        setup_fonts(&cc.egui_ctx);

        let fl_running = Arc::new(AtomicBool::new(system::fl_studio_running()));
        {
            let flag = fl_running.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(2));
                    let now = system::fl_studio_running();
                    if flag.swap(now, Ordering::Relaxed) != now {
                        ctx.request_repaint();
                    }
                }
            });
        }

        let mut app = App {
            lang: settings.language,
            pal: if settings.dark { DARK } else { LIGHT },
            settings,
            db: Arc::new(Mutex::new(HashDb::load(&hashdb::db_path()))),
            doc: None,
            tags_candidates: system::find_tags_candidates(),
            job: None,
            last_search: None,
            filter: Filter::ToFix,
            query: String::new(),
            selected: None,
            scroll_to: None,
            dialog: Dialog::None,
            fl_running,
            load_error: None,
        };
        apply_style(&cc.egui_ctx, &app.pal, app.settings.dark);

        // Cartelle (o il file Tags) trascinate sull'icona del programma
        for a in std::env::args_os().skip(1).map(PathBuf::from) {
            if a.is_dir() {
                app.add_folder(a);
            } else if a.is_file() {
                app.settings.tags_path = a.to_string_lossy().into_owned();
            }
        }

        if app.settings.tags_path.is_empty() || !Path::new(&app.settings.tags_path).is_file() {
            if let Some(c) = app.tags_candidates.first() {
                app.settings.tags_path = c.to_string_lossy().into_owned();
            }
        }
        if !app.settings.tags_path.is_empty() {
            app.start_load(&cc.egui_ctx);
        }
        app
    }

    fn l(&self, en: &'static str, it: &'static str) -> &'static str {
        self.lang.t(en, it)
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn start_load(&mut self, ctx: &egui::Context) {
        let path = PathBuf::from(&self.settings.tags_path);
        if !path.is_file() {
            self.load_error = Some(format!("{} {}", self.l("File not found:", "File non trovato:"), path.display()));
            self.doc = None;
            return;
        }
        let progress = Arc::new(Progress::default());
        let (tx, rx) = channel();
        let (db, p2, ctx2, use_hash) = (self.db.clone(), progress.clone(), ctx.clone(), self.settings.use_hash);
        std::thread::spawn(move || {
            let r = engine::load(&path, &db, &p2, use_hash);
            let _ = tx.send(JobResult::Loaded(r));
            ctx2.request_repaint();
        });
        self.job = Some(Job { kind: JobKind::Load, progress, rx });
    }

    fn start_search(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        if self.settings.folders.is_empty() {
            self.dialog = Dialog::Info {
                title: self.l("No scan location", "Nessuna posizione di scansione").into(),
                body: self
                    .l(
                        "Add at least one folder where your samples are stored (step 2).",
                        "Aggiungi almeno una cartella in cui si trovano i tuoi sample (passo 2).",
                    )
                    .into(),
                good: false,
            };
            return;
        }
        if doc.infos.iter().all(|i| i.exists) {
            self.dialog = Dialog::Info {
                title: self.l("Everything is in order", "Tutto in ordine").into(),
                body: self
                    .l("All favorites are valid. There is nothing to repair.", "Tutti i preferiti sono validi. Non c'è nulla da riparare.")
                    .into(),
                good: true,
            };
            return;
        }
        let progress = Arc::new(Progress::default());
        let (tx, rx) = channel();
        let infos = doc.infos.clone();
        let folders = self.settings.folders.clone();
        let (db, p2, ctx2, use_hash) = (self.db.clone(), progress.clone(), ctx.clone(), self.settings.use_hash);
        std::thread::spawn(move || {
            let out = engine::search(&infos, &folders, &db, &p2, use_hash);
            let _ = tx.send(JobResult::Searched(out));
            ctx2.request_repaint();
        });
        self.job = Some(Job { kind: JobKind::Search, progress, rx });
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else { return };
        let Ok(res) = job.rx.try_recv() else { return };
        self.job = None;
        match res {
            JobResult::Loaded(Ok(l)) => {
                if self.settings.folders.is_empty() {
                    self.settings.folders = engine::suggest_folders(&l.infos);
                }
                self.doc = Some(Doc {
                    auto_rows: l.rows.clone(),
                    tags: l.tags,
                    infos: l.infos,
                    rows: l.rows,
                    searched: false,
                });
                self.load_error = None;
                self.selected = None;
                self.last_search = None;
            }
            JobResult::Loaded(Err(e)) => {
                self.doc = None;
                self.load_error = Some(format!("{} {e}", self.l("Unable to read the file:", "Impossibile leggere il file:")));
            }
            JobResult::Searched(out) => {
                if let Some(doc) = &mut self.doc {
                    doc.auto_rows = out.rows.clone();
                    doc.rows = out.rows;
                    doc.searched = true;
                }
                self.last_search = Some((out.dirs, out.files, out.elapsed, out.cancelled));
                self.filter = Filter::ToFix;
                self.selected = None;
            }
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
        });
        for p in dropped {
            if p.is_dir() {
                self.add_folder(p);
            } else if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Tags")) && !self.busy() {
                self.settings.tags_path = p.to_string_lossy().into_owned();
                self.start_load(ctx);
            }
        }
    }

    fn add_folder(&mut self, p: PathBuf) {
        let k = norm_key(&p.to_string_lossy());
        if !self.settings.folders.iter().any(|f| norm_key(&f.to_string_lossy()) == k) {
            self.settings.folders.push(p);
        }
    }

    fn try_apply(&mut self) {
        let Some(doc) = &self.doc else { return };
        let s = plan::summary(&doc.rows);
        if self.settings.test_mode {
            self.dialog = Dialog::Simulation(s);
        } else if self.fl_running.load(Ordering::Relaxed) {
            self.dialog = Dialog::FlOpen;
        } else {
            self.dialog = Dialog::ConfirmApply(s);
        }
    }

    fn do_apply(&mut self, ctx: &egui::Context) {
        let Some(doc) = &self.doc else { return };
        if system::fl_studio_running() {
            self.dialog = Dialog::FlOpen;
            return;
        }
        match engine::apply(&doc.tags, &doc.infos, &doc.rows, &self.db) {
            Ok(backup) => {
                let body = self.lang.f(
                    "The favorites file has been updated.\n\nBackup of the previous version:\n{0}\n\nOpen FL Studio to review your favorites.",
                    "Il file dei preferiti è stato aggiornato.\n\nBackup della versione precedente:\n{0}\n\nApri FL Studio per controllare i preferiti.",
                    &[&backup.display()],
                );
                self.dialog = Dialog::Info { title: self.l("Changes applied", "Modifiche applicate").into(), body, good: true };
                self.start_load(ctx);
            }
            Err(e) => {
                self.dialog = Dialog::Info {
                    title: self.l("Unable to save", "Salvataggio non riuscito").into(),
                    body: format!(
                        "{}\n\n{e}",
                        self.l("The original file has not been modified.", "Il file originale non è stato modificato.")
                    ),
                    good: false,
                };
            }
        }
    }

    fn visible_rows(&self) -> Vec<usize> {
        let Some(doc) = &self.doc else { return vec![] };
        let q = self.query.trim().to_lowercase();
        (0..doc.rows.len())
            .filter(|&i| self.filter.accepts(&doc.rows[i]))
            .filter(|&i| {
                q.is_empty()
                    || doc.infos[i].path.to_lowercase().contains(&q)
                    || doc.rows[i].new_path().is_some_and(|p| p.to_lowercase().contains(&q))
            })
            .collect()
    }
}

// --------------------------------------------------------------------------
// Stile
// --------------------------------------------------------------------------

fn setup_fonts(ctx: &egui::Context) {
    // Inter (licenza OFL), incluso nell'eseguibile: stesso aspetto su Windows e macOS.
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "inter_semibold".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.ttf"))),
    );
    // Cinese dell'interfaccia: Noto Sans SC (OFL) ridotto ai soli caratteri usati in i18n.rs.
    // Se cambiano le traduzioni cinesi va rigenerato (vedi assets/fonts/README.md).
    fonts.font_data.insert(
        "cjk_ui".into(),
        Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/NotoSansSC-UI.otf"))),
    );
    let mut cjk = vec!["cjk_ui".to_string()];
    // Font CJK completo del sistema, se c'e: serve per i nomi dei sample in cinese/giapponese/coreano.
    if let Some(data) = system_cjk_font() {
        fonts.font_data.insert("cjk_system".into(), Arc::new(egui::FontData::from_owned(data)));
        cjk.push("cjk_system".into());
    }

    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "inter".into());
    let mut bold_family = vec!["inter_semibold".to_string()];
    bold_family.extend(fallback);
    fonts.families.insert(FontFamily::Name("bold".into()), bold_family);
    for family in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name("bold".into())] {
        fonts.families.entry(family).or_default().extend(cjk.iter().cloned());
    }
    ctx.set_fonts(fonts);
}

fn system_cjk_font() -> Option<Vec<u8>> {
    let candidates: &[&str] = if cfg!(windows) {
        &[r"C:\Windows\Fonts\msyh.ttc", r"C:\Windows\Fonts\msyh.ttf", r"C:\Windows\Fonts\simsun.ttc"]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
        ]
    } else {
        &["/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"]
    };
    candidates.iter().find_map(|p| std::fs::read(p).ok())
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

fn apply_style(ctx: &egui::Context, p: &Palette, dark: bool) {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.panel;
    v.window_fill = p.card;
    v.extreme_bg_color = if dark { Color32::from_rgb(0x12, 0x14, 0x18) } else { Color32::from_rgb(0xf0, 0xf2, 0xf6) };
    v.faint_bg_color = if dark { Color32::from_rgb(0x23, 0x27, 0x30) } else { Color32::from_rgb(0xf6, 0xf7, 0xfa) };
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = tint(p.accent, if dark { 60 } else { 50 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;
    let r = CornerRadius::same(8);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.bg_fill = p.card_hi;
    v.widgets.inactive.weak_bg_fill = p.card_hi;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.bg_fill = tint(p.accent, 40);
    v.widgets.hovered.weak_bg_fill = if dark { Color32::from_rgb(0x34, 0x3a, 0x46) } else { Color32::from_rgb(0xe4, 0xe7, 0xee) };
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, tint(p.accent, 160));
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, p.text);
    v.widgets.active.weak_bg_fill = tint(p.accent, 70);
    v.widgets.active.bg_fill = tint(p.accent, 90);
    v.widgets.active.fg_stroke = Stroke::new(1.5, p.text);
    v.striped = true;

    // Il tema lo decido io, non quello del sistema: altrimenti i colori si mescolano.
    let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
    ctx.set_theme(theme);
    ctx.set_visuals_of(theme, v);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 7.0);
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.scroll.bar_width = 8.0;
        s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
        s.text_styles.insert(TextStyle::Heading, bold(19.0));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
    });
}

// --------------------------------------------------------------------------
// Pezzi di interfaccia riutilizzabili
// --------------------------------------------------------------------------

fn card(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    Frame::new()
        .fill(tint(color, 38))
        .stroke(Stroke::new(1.0, tint(color, 110)))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(9, 2))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).color(color).font(bold(12.0))).wrap_mode(egui::TextWrapMode::Extend))
        })
        .response
}

fn step_title(ui: &mut egui::Ui, p: &Palette, n: &str, title: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 12.0, p.accent);
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, n, bold(13.0), p.on_accent);
        ui.label(RichText::new(title).font(bold(15.5)));
    });
}

fn hint(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).color(p.weak).size(12.5));
}

fn accent_button(p: &Palette, text: &str, enabled_look: bool) -> egui::Button<'static> {
    let fill = if enabled_look { p.accent } else { p.card_hi };
    let color = if enabled_look { p.on_accent } else { p.weak };
    egui::Button::new(RichText::new(text.to_string()).font(bold(14.5)).color(color))
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(10))
        .min_size(Vec2::new(0.0, 38.0))
}

fn stat_tile(ui: &mut egui::Ui, p: &Palette, value: usize, label: &str, color: Color32, width: f32) {
    Frame::new()
        .fill(tint(color, 22))
        .stroke(Stroke::new(1.0, tint(color, 70)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 7))
        .show(ui, |ui| {
            ui.set_width((width - 18.0).max(40.0));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(fmt_num(value as u64)).font(bold(21.0)).color(color));
                ui.add(egui::Label::new(RichText::new(label).color(p.weak).size(11.5)).truncate());
            });
        });
}

fn fmt_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{b:.0} B")
    }
}

fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn app_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).unwrap_or_default()
}

pub fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("{APP_NAME} - {APP_SUBTITLE}"))
            .with_app_id(crate::engine::APP_ID)
            .with_inner_size([1320.0, 840.0])
            .with_min_inner_size([880.0, 560.0])
            .with_resizable(true)
            .with_drag_and_drop(true)
            .with_icon(app_icon()),
        ..Default::default()
    }
}

// --------------------------------------------------------------------------
// Disegno
// --------------------------------------------------------------------------

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_job();
        self.handle_drops(&ctx);
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        self.keyboard(&ctx);

        let p = self.pal;
        egui::Panel::top("top")
            .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(18, 12)))
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("bottom")
            .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(18, 12)).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| self.apply_bar(ui));
        // Barra laterale: si allarga o si stringe trascinando il bordo destro
        egui::Panel::left("left")
            .resizable(true)
            .default_size(360.0)
            .min_size(290.0)
            .max_size(560.0)
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 18, right: 10, top: 4, bottom: 12 }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.left_panel(ui, &ctx));
            });
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 8, right: 18, top: 4, bottom: 12 }))
            .show(ui, |ui| self.central(ui, &ctx));

        self.dialogs(&ctx);
        self.drop_overlay(&ctx);
    }
}

impl App {
    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || !matches!(self.dialog, Dialog::None) {
            return;
        }
        let (down, up, esc) =
            ctx.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Escape)));
        if esc {
            self.selected = None;
        }
        if !(down || up) {
            return;
        }
        let vis = self.visible_rows();
        if vis.is_empty() {
            return;
        }
        let pos = self.selected.and_then(|s| vis.iter().position(|&v| v == s));
        let next = match (pos, down) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(vis.len() - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.selected = Some(vis[next]);
        self.scroll_to = Some(next);
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(10), tint(p.accent, 40));
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "★", FontId::proportional(24.0), p.accent);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(APP_NAME).font(bold(20.0)));
                    ui.label(RichText::new(APP_SUBTITLE).color(p.weak).size(14.0));
                    chip(ui, &format!("v{APP_VERSION}"), p.weak);
                });
                ui.label(
                    RichText::new(self.l(
                        "Locate moved samples and restore your FL Studio browser favorites",
                        "Ritrova i sample spostati e ripristina i preferiti del browser di FL Studio",
                    ))
                    .color(p.weak)
                    .size(12.5),
                );
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let icon = if self.settings.dark { "☀" } else { "🌙" };
                if ui
                    .add(egui::Button::new(RichText::new(icon).size(16.0)).min_size(Vec2::splat(34.0)))
                    .on_hover_text(self.l("Light / dark theme", "Tema chiaro / scuro"))
                    .clicked()
                {
                    self.settings.dark = !self.settings.dark;
                    self.pal = if self.settings.dark { DARK } else { LIGHT };
                    apply_style(ui.ctx(), &self.pal, self.settings.dark);
                }
                // selettore lingua
                let label = RichText::new(format!("🌐  {}  ⏷", self.lang.code())).font(bold(12.5));
                ui.menu_button(label, |ui| {
                    for l in Lang::ALL {
                        if ui.selectable_label(self.lang == l, RichText::new(l.name()).size(14.0)).clicked() {
                            self.lang = l;
                            self.settings.language = l;
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text(self.l("Language", "Lingua"));
                ui.add_space(6.0);
                if self.fl_running.load(Ordering::Relaxed) {
                    chip(
                        ui,
                        self.l("FL Studio is running. Close it before applying changes", "FL Studio è aperto. Chiudilo prima di applicare"),
                        p.red,
                    );
                } else {
                    chip(ui, self.l("FL Studio is closed", "FL Studio è chiuso"), p.green);
                }
            });
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.pal;
        let w = ui.available_width();

        // ---------------- 1. File Tags ----------------
        card(&p).show(ui, |ui| {
            ui.set_width(w - 30.0);
            step_title(ui, &p, "1", self.l("Favorites file", "File dei preferiti"));
            ui.add_space(2.0);
            let shown = if self.settings.tags_path.is_empty() {
                self.l("No file selected", "Nessun file selezionato").to_string()
            } else {
                self.settings.tags_path.clone()
            };
            Frame::new().fill(p.card_hi).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 7)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(&shown).size(12.0).color(p.weak)).truncate()).on_hover_text(&shown);
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!self.busy(), |ui| {
                    if ui.button(self.l("📂  Browse…", "📂  Sfoglia…")).clicked() {
                        let mut dlg = rfd::FileDialog::new()
                            .set_title(self.l("Select the FL Studio Tags file", "Seleziona il file Tags di FL Studio"));
                        if let Some(dir) = Path::new(&self.settings.tags_path).parent().filter(|d| d.is_dir()) {
                            dlg = dlg.set_directory(dir);
                        }
                        if let Some(f) = dlg.pick_file() {
                            self.settings.tags_path = f.to_string_lossy().into_owned();
                            self.start_load(ctx);
                        }
                    }
                    if ui
                        .button(self.l("⟳  Reload", "⟳  Ricarica"))
                        .on_hover_text(self.l("Read the file again and re-check every favorite", "Rileggi il file e ricontrolla ogni preferito"))
                        .clicked()
                    {
                        self.start_load(ctx);
                    }
                    if self.tags_candidates.len() > 1 {
                        let mut pick = None;
                        ui.menu_button(self.l("Other…", "Altri…"), |ui| {
                            for c in &self.tags_candidates {
                                if ui.button(c.to_string_lossy()).clicked() {
                                    pick = Some(c.clone());
                                }
                            }
                        });
                        if let Some(c) = pick {
                            self.settings.tags_path = c.to_string_lossy().into_owned();
                            self.start_load(ctx);
                        }
                    }
                });
            });
            if let Some(e) = &self.load_error {
                ui.label(RichText::new(e).color(p.red).size(12.5));
                if self.tags_candidates.is_empty() {
                    hint(
                        ui,
                        &p,
                        self.l(
                            "It is usually located in Documents/Image-Line/…/Settings/Browser/Tags",
                            "Di solito si trova in Documenti/Image-Line/…/Settings/Browser/Tags",
                        ),
                    );
                }
            }

            if let Some(doc) = &self.doc {
                ui.add_space(4.0);
                let s = plan::summary(&doc.rows);
                let tw = (ui.available_width() - 16.0) / 3.0;
                ui.horizontal(|ui| {
                    stat_tile(ui, &p, s.total, self.l("Total", "Totali"), p.text, tw);
                    stat_tile(ui, &p, s.working, self.l("Valid", "Validi"), p.green, tw);
                    stat_tile(ui, &p, s.missing, self.l("Missing", "Mancanti"), if s.missing > 0 { p.yellow } else { p.gray }, tw);
                });
                if doc.searched || s.update + s.remove > 0 {
                    ui.horizontal(|ui| {
                        stat_tile(ui, &p, s.update, self.l("To relink", "Da ricollegare"), p.green, tw);
                        stat_tile(ui, &p, s.remove, self.l("To remove", "Da rimuovere"), p.red, tw);
                        stat_tile(ui, &p, s.check, self.l("To review", "Da verificare"), p.orange, tw);
                    });
                }
            }
            if let Some(job) = self.job.as_ref().filter(|j| j.kind == JobKind::Load) {
                let done = job.progress.hashed_files.load(Ordering::Relaxed);
                let total = job.progress.to_hash.load(Ordering::Relaxed).max(1);
                let txt = self.lang.f(
                    "Checking favorites: {0} of {1}",
                    "Controllo dei preferiti: {0} di {1}",
                    &[&fmt_num(done), &fmt_num(total)],
                );
                ui.add(egui::ProgressBar::new(done as f32 / total as f32).fill(p.accent).text(txt));
                hint(
                    ui,
                    &p,
                    self.l(
                        "The first run reads every file; later runs are much faster.",
                        "La prima volta legge tutti i file, poi è molto più veloce.",
                    ),
                );
            }
        });

        ui.add_space(10.0);

        // ---------------- 2. Cartelle ----------------
        card(&p).show(ui, |ui| {
            ui.set_width(w - 30.0);
            step_title(ui, &p, "2", self.l("Scan locations", "Posizioni di scansione"));
            hint(
                ui,
                &p,
                self.l(
                    "Folders where your samples are stored now. Subfolders are included.",
                    "Le cartelle in cui si trovano ora i tuoi sample. Sono incluse le sottocartelle.",
                ),
            );
            ui.add_space(2.0);

            let mut remove = None;
            let lang = self.lang;
            let busy = self.busy();
            Frame::new().fill(p.card_hi).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.settings.folders.is_empty() {
                    ui.label(
                        RichText::new(lang.t("No folders yet. Add one or drop it here.", "Nessuna cartella. Aggiungine una o trascinala qui."))
                            .color(p.weak)
                            .size(12.5),
                    );
                }
                egui::ScrollArea::vertical().max_height(170.0).show(ui, |ui| {
                    for (i, f) in self.settings.folders.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let s = f.to_string_lossy();
                            let ok = f.is_dir();
                            ui.label(RichText::new("📁").color(if ok { p.accent } else { p.red }));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add_enabled(!busy, egui::Button::new("✖").small().frame(false))
                                    .on_hover_text(lang.t("Remove this folder", "Togli questa cartella"))
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    let txt = if ok {
                                        RichText::new(s.as_ref())
                                    } else {
                                        RichText::new(format!("{s} ({})", lang.t("unavailable", "non disponibile"))).color(p.red)
                                    };
                                    ui.add(egui::Label::new(txt.size(12.5)).truncate()).on_hover_text(s.as_ref());
                                });
                            });
                        });
                    }
                });
            });
            if let Some(i) = remove {
                self.settings.folders.remove(i);
            }
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if ui.button(lang.t("➕  Add folder…", "➕  Aggiungi cartella…")).clicked() {
                        if let Some(dirs) = rfd::FileDialog::new()
                            .set_title(lang.t("Select scan locations", "Seleziona le posizioni di scansione"))
                            .pick_folders()
                        {
                            for d in dirs {
                                self.add_folder(d);
                            }
                        }
                    }
                    if !self.settings.folders.is_empty() && ui.button(lang.t("Clear", "Svuota")).clicked() {
                        self.settings.folders.clear();
                    }
                });
            });

            ui.add_space(4.0);
            let tip = lang.t(
                "FLeeker keeps a content signature of every valid favorite. If a sample is later moved and renamed, it can still be identified. It also tells apart different samples that share the same file name.",
                "FLeeker memorizza una firma del contenuto di ogni preferito valido. Se in seguito un sample viene spostato e rinominato, può comunque essere identificato. Distingue anche sample diversi con lo stesso nome file.",
            );
            ui.add_enabled_ui(!busy, |ui| {
                ui.checkbox(
                    &mut self.settings.use_hash,
                    lang.t("Detect renamed files (content matching)", "Rileva i file rinominati (confronto del contenuto)"),
                )
                .on_hover_text(tip);
            });
            let n_fp = self.db.lock().unwrap().fingerprints.len();
            let idx = lang.f("Content index: {0} samples", "Indice dei contenuti: {0} sample", &[&fmt_num(n_fp as u64)]);
            hint(ui, &p, &idx);

            ui.add_space(6.0);
            let searching = self.job.as_ref().is_some_and(|j| j.kind == JobKind::Search);
            if searching {
                let pr = self.job.as_ref().unwrap().progress.clone();
                let dirs = pr.dirs.load(Ordering::Relaxed);
                let files = pr.files.load(Ordering::Relaxed);
                let to_hash = pr.to_hash.load(Ordering::Relaxed);
                let hashed = pr.hashed_files.load(Ordering::Relaxed);
                if to_hash > 0 {
                    let frac = hashed as f32 / to_hash as f32;
                    let txt = lang.f(
                        "Verifying content: {0} of {1}",
                        "Verifica del contenuto: {0} di {1}",
                        &[&fmt_num(hashed), &fmt_num(to_hash)],
                    );
                    ui.add(egui::ProgressBar::new(frac).fill(p.accent).text(txt));
                } else {
                    let txt = lang.f("Folders: {0}  ·  Files: {1}", "Cartelle: {0}  ·  File: {1}", &[&fmt_num(dirs), &fmt_num(files)]);
                    ui.add(egui::ProgressBar::new(0.0).animate(true).fill(p.accent).text(txt));
                }
                if ui
                    .add(egui::Button::new(lang.t("■  Stop scan", "■  Interrompi scansione")).min_size(Vec2::new(ui.available_width(), 34.0)))
                    .clicked()
                {
                    pr.cancel.store(true, Ordering::Relaxed);
                }
            } else {
                let can = self.doc.is_some() && !busy;
                let r = ui.add_enabled(
                    can,
                    accent_button(&p, lang.t("🔍  Scan for missing samples", "🔍  Cerca i sample mancanti"), can)
                        .min_size(Vec2::new(ui.available_width(), 42.0)),
                );
                if r.clicked() {
                    self.start_search(ctx);
                }
            }
            if let Some((dirs, files, el, cancelled)) = self.last_search {
                let mut msg = lang.f(
                    "Scanned {0} folders and {1} files in {2} s",
                    "Analizzate {0} cartelle e {1} file in {2} s",
                    &[&fmt_num(dirs), &fmt_num(files), &format!("{:.1}", el.as_secs_f32())],
                );
                if cancelled {
                    msg = format!("{} {msg}", lang.t("Scan stopped early.", "Scansione interrotta."));
                }
                hint(ui, &p, &msg);
            }
        });

        ui.add_space(10.0);
        Frame::new()
            .fill(tint(p.blue, 18))
            .stroke(Stroke::new(1.0, tint(p.blue, 60)))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(w - 26.0);
                ui.label(RichText::new(self.l("ℹ  Good to know", "ℹ  Da sapere")).font(bold(13.0)).color(p.blue));
                hint(
                    ui,
                    &p,
                    self.l(
                        "FL Studio's Starred view only lists favorites located inside the folders set in Options › File settings › Browser extra search folders. If a relinked favorite does not appear, add its folder there.",
                        "La vista Starred di FL Studio mostra solo i preferiti che si trovano nelle cartelle impostate in Options › File settings › Browser extra search folders. Se un preferito ricollegato non compare, aggiungi lì la sua cartella.",
                    ),
                );
            });
    }

    fn central(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = self.pal;
        let lang = self.lang;
        let Some(doc) = &self.doc else {
            card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(ui.available_height());
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() * 0.3);
                    ui.label(RichText::new("★").size(56.0).color(tint(p.accent, 180)));
                    if self.busy() {
                        ui.spinner();
                        ui.label(lang.t("Loading favorites…", "Caricamento dei preferiti…"));
                    } else {
                        ui.label(
                            RichText::new(lang.t("Select the Tags file to get started", "Seleziona il file Tags per iniziare")).font(bold(17.0)),
                        );
                        hint(
                            ui,
                            &p,
                            lang.t(
                                "Click \"Browse…\" on the left, or drop the Tags file onto this window.",
                                "Premi \"Sfoglia…\" a sinistra, oppure trascina il file Tags su questa finestra.",
                            ),
                        );
                    }
                });
            });
            return;
        };

        // ---- barra filtri ----
        let counts: Vec<(Filter, usize)> =
            Filter::ALL.into_iter().map(|f| (f, doc.rows.iter().filter(|r| f.accepts(r)).count())).collect();

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button(lang.t("Bulk actions  ⏷", "Azioni rapide  ⏷"), |ui| self.quick_actions(ui));
                ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text(lang.t("🔎  Filter by name…", "🔎  Filtra per nome…"))
                        .desired_width(190.0),
                );
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    egui::ScrollArea::horizontal().id_salt("filters").show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (f, n) in &counts {
                                if *n == 0 && !matches!(f, Filter::ToFix | Filter::All) && self.filter != *f {
                                    continue;
                                }
                                let sel = self.filter == *f;
                                let txt = RichText::new(format!("{}  {}", f.label(lang), fmt_num(*n as u64))).size(13.0);
                                let b = egui::Button::new(if sel { txt.color(p.on_accent).strong() } else { txt })
                                    .fill(if sel { p.accent } else { p.card })
                                    .stroke(Stroke::new(1.0, if sel { p.accent } else { p.border }))
                                    .corner_radius(CornerRadius::same(255));
                                if ui.add(b).clicked() {
                                    self.filter = *f;
                                    self.selected = None;
                                }
                            }
                        });
                    });
                });
            });
        });
        ui.add_space(6.0);

        // ---- pannello dettagli: si ridimensiona trascinando il bordo superiore ----
        if self.selected.is_some() {
            let max = (ui.available_height() - 160.0).max(160.0);
            egui::Panel::bottom("detail")
                .resizable(true)
                .default_size(250.0)
                .min_size(150.0)
                .max_size(max)
                .frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 8, bottom: 0 }))
                .show(ui, |ui| self.detail(ui, ctx));
        }

        let Some(doc) = &self.doc else { return };
        let vis = self.visible_rows();
        let table_h = ui.available_height().max(120.0);
        let mut clicked: Option<usize> = None;
        let mut cycle: Option<usize> = None;

        card(&p).inner_margin(Margin::same(6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(table_h - 14.0);
            if vis.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(table_h * 0.3);
                    ui.label(RichText::new("✔").size(40.0).color(p.green));
                    let msg = if self.filter == Filter::ToFix && !doc.searched {
                        if doc.infos.iter().all(|i| i.exists) {
                            lang.t("All favorites are valid. Nothing to repair.", "Tutti i preferiti sono validi. Nulla da riparare.")
                        } else {
                            lang.t("Click \"Scan for missing samples\" to begin.", "Premi \"Cerca i sample mancanti\" per iniziare.")
                        }
                    } else {
                        lang.t("No favorites in this view.", "Nessun preferito in questa vista.")
                    };
                    ui.label(RichText::new(msg).font(bold(15.0)));
                });
                return;
            }
            // Colonne ridimensionabili trascinando i bordi dell'intestazione
            let mut tb = TableBuilder::new(ui)
                .id_salt("tabella")
                .striped(true)
                .resizable(true)
                .sense(Sense::click())
                .cell_layout(Layout::left_to_right(Align::Center))
                .auto_shrink([false, false])
                .max_scroll_height(table_h - 50.0)
                .column(Column::exact(14.0).resizable(false))
                .column(Column::initial(280.0).at_least(120.0).clip(true))
                .column(Column::initial(140.0).at_least(90.0).clip(true))
                .column(Column::initial(108.0).at_least(90.0).clip(true))
                .column(Column::remainder().at_least(160.0).clip(true));
            if let Some(i) = self.scroll_to.take() {
                tb = tb.scroll_to_row(i, None);
            }
            tb.header(28.0, |mut h| {
                h.col(|_| {});
                for t in [
                    lang.t("Favorite", "Preferito"),
                    lang.t("Status", "Stato"),
                    lang.t("Action", "Azione"),
                    lang.t("New location / details", "Nuova posizione / dettagli"),
                ] {
                    h.col(|ui| {
                        ui.label(RichText::new(t).color(p.weak).font(bold(12.0)));
                    });
                }
            })
            .body(|body| {
                body.rows(32.0, vis.len(), |mut row| {
                    let i = vis[row.index()];
                    let r = &doc.rows[i];
                    let info = &doc.infos[i];
                    row.set_selected(self.selected == Some(i));
                    let sc = status_color(&p, r.status);
                    row.col(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(4.0, 20.0), Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(2), sc);
                    });
                    row.col(|ui| {
                        let name = basename_any(&info.path);
                        let txt = if r.action == Action::Remove {
                            RichText::new(name).strikethrough().color(p.weak)
                        } else {
                            RichText::new(name)
                        };
                        ui.add(egui::Label::new(txt).truncate().selectable(false)).on_hover_text(&info.path);
                    });
                    row.col(|ui| {
                        chip(ui, status_label(lang, r.status), sc);
                    });
                    row.col(|ui| {
                        let ac = action_color(&p, r.action);
                        let b = egui::Button::new(RichText::new(action_label(lang, r.action)).color(ac).font(bold(12.5)))
                            .fill(tint(ac, 26))
                            .stroke(Stroke::new(1.0, tint(ac, 90)))
                            .corner_radius(CornerRadius::same(6))
                            .min_size(Vec2::new(88.0, 24.0));
                        if ui.add(b).on_hover_text(lang.t("Click to change the action", "Clic per cambiare azione")).clicked() {
                            cycle = Some(i);
                        }
                    });
                    row.col(|ui| {
                        let (txt, col) = match r.new_path() {
                            Some(np) => (format!("➡  {np}"), p.text),
                            None => (reason_text(lang, &r.reason), p.weak),
                        };
                        ui.add(egui::Label::new(RichText::new(&txt).color(col).size(12.5)).truncate().selectable(false))
                            .on_hover_text(&txt);
                    });
                    if row.response().clicked() {
                        clicked = Some(i);
                    }
                    if row.response().double_clicked() {
                        cycle = Some(i);
                    }
                });
            });
        });

        if let Some(i) = clicked {
            self.selected = if self.selected == Some(i) { None } else { Some(i) };
        }
        if let Some(i) = cycle {
            self.selected = Some(i);
            if let Some(doc) = &mut self.doc {
                let r = &mut doc.rows[i];
                let order: Vec<Action> = if r.candidates.is_empty() {
                    vec![Action::Keep, Action::Remove]
                } else {
                    vec![Action::Update, Action::Remove, Action::Keep]
                };
                let pos = order.iter().position(|a| *a == r.action).map(|x| x + 1).unwrap_or(0);
                r.action = order[pos % order.len()];
            }
        }
    }

    fn quick_actions(&mut self, ui: &mut egui::Ui) {
        let lang = self.lang;
        let Some(doc) = &mut self.doc else { return };
        let n_nf = doc.rows.iter().filter(|r| r.status == Status::NotFound && r.action == Action::Keep).count();
        let n_dup = doc.rows.iter().filter(|r| r.status == Status::OkSameContent && r.action == Action::Keep).count();
        let t1 = lang.f("Remove all not-found favorites ({0})", "Rimuovi tutti i preferiti non trovati ({0})", &[&n_nf]);
        if ui
            .add_enabled(n_nf > 0, egui::Button::new(t1))
            .on_hover_text(lang.t("Favorites on disconnected drives are not affected", "I preferiti su unità scollegate non vengono toccati"))
            .clicked()
        {
            for r in &mut doc.rows {
                if r.status == Status::NotFound && r.action == Action::Keep {
                    r.action = Action::Remove;
                }
            }
            ui.close();
        }
        let t2 = lang.f("Remove duplicate-content favorites ({0})", "Rimuovi i preferiti con contenuto duplicato ({0})", &[&n_dup]);
        if ui
            .add_enabled(n_dup > 0, egui::Button::new(t2))
            .on_hover_text(lang.t(
                "Different favorites pointing to identical files: the first one is kept",
                "Preferiti diversi che puntano a file identici: viene mantenuto il primo",
            ))
            .clicked()
        {
            for r in &mut doc.rows {
                if r.status == Status::OkSameContent && r.action == Action::Keep {
                    r.action = Action::Remove;
                }
            }
            ui.close();
        }
        ui.separator();
        if ui.button(lang.t("Restore automatic suggestions", "Ripristina i suggerimenti automatici")).clicked() {
            doc.rows = doc.auto_rows.clone();
            ui.close();
        }
    }

    fn detail(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let p = self.pal;
        let lang = self.lang;
        let Some(i) = self.selected else { return };
        let Some(doc) = &mut self.doc else { return };
        if i >= doc.rows.len() {
            self.selected = None;
            return;
        }
        let info = doc.infos[i].clone();
        let mut close = false;
        // Il contenuto scorre: cosi il pannello si puo stringere quanto si vuole
        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            egui::ScrollArea::vertical().id_salt("detail_scroll").auto_shrink([false, false]).show(ui, |ui| {
            let r = &mut doc.rows[i];
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(basename_any(&info.path)).font(bold(15.5))).truncate());
                chip(ui, status_label(lang, r.status), status_color(&p, r.status));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(egui::Button::new("✖").frame(false)).on_hover_text(lang.t("Close (Esc)", "Chiudi (Esc)")).clicked() {
                        close = true;
                    }
                    for a in [Action::Keep, Action::Remove, Action::Update] {
                        if a == Action::Update && r.candidates.is_empty() {
                            continue;
                        }
                        let sel = r.action == a;
                        let ac = action_color(&p, a);
                        let b = egui::Button::new(
                            RichText::new(action_label(lang, a)).color(if sel { p.on_accent } else { ac }).font(bold(12.5)),
                        )
                        .fill(if sel { ac } else { tint(ac, 20) })
                        .stroke(Stroke::new(1.0, tint(ac, 120)))
                        .corner_radius(CornerRadius::same(7));
                        if ui.add(b).clicked() {
                            r.action = a;
                        }
                    }
                    ui.label(RichText::new(lang.t("Action:", "Azione:")).color(p.weak));
                });
            });
            ui.label(RichText::new(reason_text(lang, &r.reason)).color(p.weak).size(12.5));

            egui::Grid::new("dettaglio").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                ui.label(RichText::new(lang.t("Original location", "Posizione originale")).color(p.weak).size(12.5));
                ui.add(egui::Label::new(RichText::new(&info.path).size(12.5)).truncate()).on_hover_text(&info.path);
                ui.end_row();
                let size = if info.exists { info.size } else { info.fingerprint.map(|f| f.1) };
                if let Some(s) = size {
                    ui.label(RichText::new(lang.t("File size", "Dimensione")).color(p.weak).size(12.5));
                    ui.label(RichText::new(fmt_bytes(s)).size(12.5));
                    ui.end_row();
                }
                if !info.exists {
                    ui.label(RichText::new(lang.t("Content signature", "Firma del contenuto")).color(p.weak).size(12.5));
                    ui.label(
                        RichText::new(if info.fingerprint.is_some() {
                            lang.t("Available: renamed copies can be identified", "Disponibile: le copie rinominate possono essere identificate")
                        } else {
                            lang.t(
                                "Not available (the file was already missing on first run)",
                                "Non disponibile (il file mancava già al primo avvio)",
                            )
                        })
                        .size(12.5),
                    );
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                if info.exists && ui.button(lang.t("📂  Show in folder", "📂  Mostra nella cartella")).clicked() {
                    system::reveal(&info.path);
                }
                let folder = parent_any(&info.path);
                if !info.exists
                    && !folder.is_empty()
                    && Path::new(folder).is_dir()
                    && ui.button(lang.t("📂  Open original folder", "📂  Apri la cartella originale")).clicked()
                {
                    system::reveal(&info.path);
                }
            });

            if !r.candidates.is_empty() {
                ui.add_space(2.0);
                let head = lang.f("Matches found ({0})", "Corrispondenze trovate ({0})", &[&r.candidates.len()]);
                ui.label(RichText::new(head).font(bold(13.0)));
                {
                    for (ci, c) in r.candidates.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let sel = r.chosen == ci && r.action == Action::Update;
                            if ui.radio(sel, "").on_hover_text(lang.t("Use this file", "Usa questo file")).clicked() {
                                r.chosen = ci;
                                r.action = Action::Update;
                            }
                            if ui
                                .add(egui::Button::new("📂").small().frame(false))
                                .on_hover_text(lang.t("Show in folder", "Mostra nella cartella"))
                                .clicked()
                            {
                                system::reveal(&c.path);
                            }
                            match c.matches_original {
                                Some(true) => {
                                    chip(ui, lang.t("Identical to original", "Identico all'originale"), p.green);
                                }
                                Some(false) => {
                                    chip(ui, lang.t("Different content", "Contenuto diverso"), p.orange);
                                }
                                None => {}
                            }
                            ui.label(RichText::new(fmt_bytes(c.size)).size(12.0).color(p.weak));
                            ui.add(egui::Label::new(RichText::new(&c.path).size(12.5)).truncate()).on_hover_text(&c.path);
                        });
                    }
                }
            }
            });
        });
        if close {
            self.selected = None;
        }
    }

    fn apply_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal;
        let lang = self.lang;
        let s = self.doc.as_ref().map(|d| plan::summary(&d.rows)).unwrap_or_default();
        let changes = s.update + s.remove;
        ui.horizontal(|ui| {
            let (label, sub, col) = if self.settings.test_mode {
                (
                    lang.t("Preview mode", "Modalità anteprima"),
                    lang.t("No changes are written. Results are shown only", "Nessuna modifica viene scritta. Vengono mostrati solo i risultati"),
                    p.blue,
                )
            } else {
                (
                    lang.t("Live mode", "Modalità reale"),
                    lang.t(
                        "The Tags file will be updated (a backup is created first)",
                        "Il file Tags verrà aggiornato (prima viene creato un backup)",
                    ),
                    p.red,
                )
            };
            toggle(ui, &p, &mut self.settings.test_mode)
                .on_hover_text(lang.t("Switch between preview and live mode", "Passa tra anteprima e modalità reale"));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(label).font(bold(13.5)).color(col));
                ui.label(RichText::new(sub).size(12.0).color(p.weak));
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let can = changes > 0 && !self.busy();
                let txt = if self.settings.test_mode {
                    lang.t("Preview changes", "Anteprima modifiche")
                } else {
                    lang.t("✔  Apply changes", "✔  Applica modifiche")
                };
                if ui.add_enabled(can, accent_button(&p, txt, can).min_size(Vec2::new(200.0, 40.0))).clicked() {
                    self.try_apply();
                }
                let has_tags = Path::new(&self.settings.tags_path).is_file();
                if ui
                    .add_enabled(
                        has_tags && !self.busy(),
                        egui::Button::new(lang.t("⟲  Backups…", "⟲  Backup…")).min_size(Vec2::new(0.0, 40.0)),
                    )
                    .on_hover_text(lang.t("View and restore backups of the Tags file", "Visualizza e ripristina i backup del file Tags"))
                    .clicked()
                {
                    self.dialog = Dialog::Backups(tags::list_backups(Path::new(&self.settings.tags_path)));
                }
                ui.add_space(12.0);
                if self.doc.is_some() {
                    let msg = if changes == 0 {
                        RichText::new(lang.t("No pending changes", "Nessuna modifica in sospeso")).color(p.weak)
                    } else {
                        RichText::new(lang.f(
                            "{0} to relink  ·  {1} to remove  ·  {2} ➡ {3} favorites",
                            "{0} da ricollegare  ·  {1} da rimuovere  ·  {2} ➡ {3} preferiti",
                            &[&s.update, &s.remove, &s.total, &(s.total - s.remove)],
                        ))
                    };
                    ui.add(egui::Label::new(msg.size(13.5)).truncate());
                }
            });
        });
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if !hovering {
            return;
        }
        let p = self.pal;
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(170));
        painter.rect_stroke(rect.shrink(16.0), CornerRadius::same(16), Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            self.l("Drop scan folders (or the Tags file) here", "Rilascia qui le cartelle da scansionare (o il file Tags)"),
            bold(21.0),
            Color32::WHITE,
        );
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let p = self.pal;
        let lang = self.lang;
        let dialog = std::mem::replace(&mut self.dialog, Dialog::None);
        let modal_frame = Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::same(22));
        let mut next = None;
        let mut keep = true;
        let bullet = |ui: &mut egui::Ui, text: String, color: Color32| {
            ui.label(RichText::new(format!("•  {text}")).color(color));
        };

        match &dialog {
            Dialog::None => keep = false,
            Dialog::Info { title, body, good } => {
                let r = egui::Modal::new(egui::Id::unique("info")).frame(modal_frame).show(ctx, |ui| {
                    ui.set_width(460.0);
                    ui.label(RichText::new(title).font(bold(17.0)).color(if *good { p.green } else { p.red }));
                    ui.add_space(4.0);
                    ui.label(body);
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| ui.add(accent_button(&p, "  OK  ", true)).clicked()).inner
                });
                if r.inner || r.should_close() {
                    keep = false;
                }
            }
            Dialog::Simulation(s) => {
                let r = egui::Modal::new(egui::Id::unique("sim")).frame(modal_frame).show(ctx, |ui| {
                    ui.set_width(500.0);
                    ui.label(RichText::new(lang.t("Preview", "Anteprima")).font(bold(17.0)).color(p.blue));
                    ui.label(RichText::new(lang.t("No files have been modified.", "Nessun file è stato modificato.")).color(p.weak));
                    ui.add_space(8.0);
                    ui.label(lang.t("If you apply these changes:", "Se applichi queste modifiche:"));
                    bullet(
                        ui,
                        lang.f(
                            "{0} favorites will be relinked to their new location",
                            "{0} preferiti verranno ricollegati alla nuova posizione",
                            &[&s.update],
                        ),
                        p.green,
                    );
                    bullet(ui, lang.f("{0} favorites will be removed", "{0} preferiti verranno rimossi", &[&s.remove]), p.red);
                    bullet(
                        ui,
                        lang.f(
                            "The file will go from {0} to {1} favorites",
                            "Il file passerà da {0} a {1} preferiti",
                            &[&s.total, &(s.total - s.remove)],
                        ),
                        p.text,
                    );
                    if s.check > 0 {
                        bullet(
                            ui,
                            lang.f(
                                "{0} need review: check them before applying",
                                "{0} sono da verificare: controllali prima di applicare",
                                &[&s.check],
                            ),
                            p.orange,
                        );
                    }
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        let ok = ui.add(accent_button(&p, lang.t("  Close  ", "  Chiudi  "), true)).clicked();
                        let real = ui.button(lang.t("Switch to live mode", "Passa alla modalità reale")).clicked();
                        (ok, real)
                    })
                    .inner
                });
                let sc = r.should_close();
                let (ok, real) = r.inner;
                if real {
                    self.settings.test_mode = false;
                }
                if ok || real || sc {
                    keep = false;
                }
            }
            Dialog::ConfirmApply(s) => {
                let r = egui::Modal::new(egui::Id::unique("confirm")).frame(modal_frame).show(ctx, |ui| {
                    ui.set_width(480.0);
                    ui.label(RichText::new(lang.t("Update the favorites file?", "Aggiornare il file dei preferiti?")).font(bold(17.0)));
                    ui.add_space(6.0);
                    bullet(ui, lang.f("{0} favorites relinked", "{0} preferiti ricollegati", &[&s.update]), p.green);
                    bullet(ui, lang.f("{0} favorites removed", "{0} preferiti rimossi", &[&s.remove]), p.red);
                    if s.check > 0 {
                        bullet(ui, lang.f("{0} still need review", "{0} ancora da verificare", &[&s.check]), p.orange);
                    }
                    ui.add_space(6.0);
                    hint(
                        ui,
                        &p,
                        lang.t(
                            "A backup of the current file is created automatically, so you can always roll back.",
                            "Viene creato automaticamente un backup del file attuale, così puoi sempre tornare indietro.",
                        ),
                    );
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        let yes = ui.add(accent_button(&p, lang.t("  Apply  ", "  Applica  "), true)).clicked();
                        let no = ui.button(lang.t("Cancel", "Annulla")).clicked();
                        (yes, no)
                    })
                    .inner
                });
                let sc = r.should_close();
                let (yes, no) = r.inner;
                if yes {
                    self.dialog = Dialog::None;
                    self.do_apply(ctx);
                    return;
                }
                if no || sc {
                    keep = false;
                }
            }
            Dialog::FlOpen => {
                let r = egui::Modal::new(egui::Id::unique("flopen")).frame(modal_frame).show(ctx, |ui| {
                    ui.set_width(460.0);
                    ui.label(RichText::new(lang.t("FL Studio is running", "FL Studio è aperto")).font(bold(17.0)).color(p.red));
                    ui.add_space(4.0);
                    ui.label(lang.t(
                        "Close FL Studio and try again. FL Studio rewrites the favorites file when it exits, which would overwrite these changes.",
                        "Chiudi FL Studio e riprova. Alla chiusura FL Studio riscrive il file dei preferiti e sovrascriverebbe queste modifiche.",
                    ));
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| ui.add(accent_button(&p, "  OK  ", true)).clicked()).inner
                });
                if r.inner || r.should_close() {
                    keep = false;
                }
            }
            Dialog::Backups(list) => {
                let r = egui::Modal::new(egui::Id::unique("backups")).frame(modal_frame).show(ctx, |ui| {
                    ui.set_width(600.0);
                    ui.label(RichText::new(lang.t("Favorites file backups", "Backup del file dei preferiti")).font(bold(17.0)));
                    hint(
                        ui,
                        &p,
                        lang.t(
                            "A backup is created every time changes are applied. Backups are stored next to the Tags file.",
                            "Viene creato un backup ogni volta che applichi delle modifiche. I backup si trovano accanto al file Tags.",
                        ),
                    );
                    ui.add_space(8.0);
                    let mut pick = None;
                    if list.is_empty() {
                        ui.label(RichText::new(lang.t("No backups yet.", "Nessun backup disponibile.")).color(p.weak));
                    }
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for b in list {
                            Frame::new().fill(p.card_hi).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let fmt = match lang {
                                        Lang::En | Lang::Zh => "%Y-%m-%d  %H:%M",
                                        Lang::De => "%d.%m.%Y  %H:%M",
                                        _ => "%d/%m/%Y  %H:%M",
                                    };
                                    let when = b.modified.map(|d| d.format(fmt).to_string()).unwrap_or_default();
                                    ui.label(RichText::new(when).font(bold(13.5)));
                                    let n = lang.f("{0} favorites", "{0} preferiti", &[&b.favorites]);
                                    ui.label(RichText::new(n).color(p.weak));
                                    if b.name.contains("before_restore") || b.name.contains("prima_del_ripristino") {
                                        chip(ui, lang.t("before a restore", "prima di un ripristino"), p.gray);
                                    }
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui.button(lang.t("Restore", "Ripristina")).clicked() {
                                            pick = Some(b.clone());
                                        }
                                    });
                                });
                            });
                        }
                    });
                    ui.add_space(10.0);
                    let close =
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| ui.button(lang.t("Close", "Chiudi")).clicked()).inner;
                    (pick, close)
                });
                let sc = r.should_close();
                let (pick, close) = r.inner;
                if let Some(b) = pick {
                    next = Some(Dialog::ConfirmRestore(b));
                } else if close || sc {
                    keep = false;
                }
            }
            Dialog::ConfirmRestore(b) => {
                let r = egui::Modal::new(egui::Id::unique("restore")).frame(modal_frame).show(ctx, |ui| {
                    ui.set_width(480.0);
                    ui.label(RichText::new(lang.t("Restore this backup?", "Ripristinare questo backup?")).font(bold(17.0)));
                    ui.add_space(4.0);
                    ui.label(format!("{}  ({} {})", b.name, b.favorites, lang.t("favorites", "preferiti")));
                    hint(
                        ui,
                        &p,
                        lang.t(
                            "The current file is saved as a new backup first, so nothing is lost.",
                            "Il file attuale viene prima salvato come nuovo backup, quindi non si perde nulla.",
                        ),
                    );
                    ui.add_space(10.0);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        let yes = ui.add(accent_button(&p, lang.t("  Restore  ", "  Ripristina  "), true)).clicked();
                        let no = ui.button(lang.t("Cancel", "Annulla")).clicked();
                        (yes, no)
                    })
                    .inner
                });
                let sc = r.should_close();
                let (yes, no) = r.inner;
                if yes {
                    if system::fl_studio_running() {
                        next = Some(Dialog::FlOpen);
                    } else {
                        let tp = PathBuf::from(&self.settings.tags_path);
                        next = Some(match tags::restore_backup(&tp, &b.path) {
                            Ok(_) => {
                                self.start_load(ctx);
                                Dialog::Info {
                                    title: lang.t("Backup restored", "Backup ripristinato").into(),
                                    body: lang.t("Favorites have been reloaded.", "I preferiti sono stati ricaricati.").into(),
                                    good: true,
                                }
                            }
                            Err(e) => Dialog::Info {
                                title: lang.t("Restore failed", "Ripristino non riuscito").into(),
                                body: e.to_string(),
                                good: false,
                            },
                        });
                    }
                } else if no || sc {
                    keep = false;
                }
            }
        }

        self.dialog = match next {
            Some(d) => d,
            None if keep => dialog,
            None => Dialog::None,
        };
    }
}

/// Interruttore anteprima / reale.
fn toggle(ui: &mut egui::Ui, p: &Palette, on: &mut bool) -> egui::Response {
    let size = Vec2::new(46.0, 26.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = if *on { p.blue } else { p.red };
    ui.painter().rect_filled(rect, CornerRadius::same(13), tint(bg, 200));
    let x = egui::lerp((rect.left() + 13.0)..=(rect.right() - 13.0), t);
    ui.painter().circle_filled(egui::pos2(x, rect.center().y), 10.0, Color32::WHITE);
    resp
}
