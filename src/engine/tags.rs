//! Lettura e scrittura del file "Tags" del browser di FL Studio.
//!
//! Ogni riga viene conservata come byte grezzi: le righe che non vengono
//! modificate si riscrivono identiche byte per byte.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Cp1252,
}

/// Un preferito: percorso + tag (di solito "*", cioe la stellina).
#[derive(Clone, Debug)]
pub struct Entry {
    /// Indice della riga nel file.
    pub line: usize,
    pub path: String,
    pub tag: String,
}

#[derive(Clone, Debug)]
pub struct TagsFile {
    pub path: PathBuf,
    /// Righe grezze, senza il fine riga.
    pub lines: Vec<Vec<u8>>,
    pub entries: Vec<Entry>,
    pub newline: &'static [u8],
    pub encoding: TextEncoding,
    pub has_bom: bool,
    pub trailing_newline: bool,
    /// FL Studio scrive i percorsi tutti in minuscolo: se il file e cosi, faccio lo stesso.
    pub lowercase_paths: bool,
}

/// Cosa fare di un preferito al momento della scrittura.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Keep,
    Remove,
    Replace(String),
}

fn decode(bytes: &[u8], enc: TextEncoding) -> String {
    match enc {
        TextEncoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        TextEncoding::Cp1252 => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}

fn encode(text: &str, enc: TextEncoding) -> Vec<u8> {
    match enc {
        TextEncoding::Utf8 => text.as_bytes().to_vec(),
        TextEncoding::Cp1252 => encoding_rs::WINDOWS_1252.encode(text).0.into_owned(),
    }
}

/// Le righe con '@' (es. "@TagCase=...") e quelle vuote sono intestazioni.
/// Le altre sono "percorso,tag" con il percorso tra virgolette se serve.
fn parse_entry(text: &str) -> Option<(String, String)> {
    let s = text.trim();
    if s.is_empty() || s.starts_with('@') {
        return None;
    }
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"')?; // riga strana: la lasciamo intatta
        let path = &rest[..end];
        let after = &rest[end + 1..];
        let tag = after.strip_prefix(',').unwrap_or(after);
        Some((path.to_string(), tag.to_string()))
    } else if let Some((path, tag)) = s.rsplit_once(',') {
        Some((path.to_string(), tag.to_string()))
    } else {
        Some((s.to_string(), String::new()))
    }
}

/// Stesso stile di FL Studio: virgolette solo se servono (spazi o virgole).
pub fn format_entry(path: &str, tag: &str) -> String {
    if path.contains(' ') || path.contains(',') {
        format!("\"{path}\",{tag}")
    } else {
        format!("{path},{tag}")
    }
}

fn split_lines<'a>(body: &'a [u8], nl: &[u8]) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    if body.is_empty() {
        return out;
    }
    let mut start = 0;
    let mut i = 0;
    while i + nl.len() <= body.len() {
        if &body[i..i + nl.len()] == nl {
            out.push(&body[start..i]);
            i += nl.len();
            start = i;
        } else {
            i += 1;
        }
    }
    out.push(&body[start..]);
    out
}

impl TagsFile {
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let data = fs::read(path)?;
        Ok(Self::parse(path.to_path_buf(), &data))
    }

    pub fn parse(path: PathBuf, data: &[u8]) -> Self {
        let has_bom = data.starts_with(&[0xEF, 0xBB, 0xBF]);
        let data = if has_bom { &data[3..] } else { data };
        let encoding = if std::str::from_utf8(data).is_ok() {
            TextEncoding::Utf8
        } else {
            TextEncoding::Cp1252
        };
        let newline: &'static [u8] = if data.windows(2).any(|w| w == b"\r\n") {
            b"\r\n"
        } else {
            b"\n"
        };
        let trailing_newline = data.ends_with(newline);
        let body = if trailing_newline { &data[..data.len() - newline.len()] } else { data };

        let lines: Vec<Vec<u8>> = split_lines(body, newline).into_iter().map(|l| l.to_vec()).collect();
        let entries: Vec<Entry> = lines
            .iter()
            .enumerate()
            .filter_map(|(i, raw)| {
                parse_entry(&decode(raw, encoding)).map(|(path, tag)| Entry { line: i, path, tag })
            })
            .collect();

        let lowercase_paths = !entries.is_empty() && entries.iter().all(|e: &Entry| e.path == e.path.to_lowercase());
        TagsFile { path, lines, entries, newline, encoding, has_bom, trailing_newline, lowercase_paths }
    }

    /// Rigenera il file. `edits` e allineato a `entries`.
    pub fn build(&self, edits: &[Edit]) -> Vec<u8> {
        let mut by_line: Vec<Option<&Edit>> = vec![None; self.lines.len()];
        for (e, ed) in self.entries.iter().zip(edits) {
            by_line[e.line] = Some(ed);
        }
        let entry_at: std::collections::HashMap<usize, &Entry> =
            self.entries.iter().map(|e| (e.line, e)).collect();

        let mut out_lines: Vec<Vec<u8>> = Vec::with_capacity(self.lines.len());
        for (i, raw) in self.lines.iter().enumerate() {
            match by_line[i] {
                Some(Edit::Remove) => {}
                Some(Edit::Replace(new_path)) => {
                    let tag = &entry_at[&i].tag;
                    let p = if self.lowercase_paths { new_path.to_lowercase() } else { new_path.clone() };
                    out_lines.push(encode(&format_entry(&p, tag), self.encoding));
                }
                _ => out_lines.push(raw.clone()),
            }
        }

        let mut data = Vec::new();
        if self.has_bom {
            data.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        for (i, l) in out_lines.iter().enumerate() {
            if i > 0 {
                data.extend_from_slice(self.newline);
            }
            data.extend_from_slice(l);
        }
        if self.trailing_newline && !out_lines.is_empty() {
            data.extend_from_slice(self.newline);
        }
        data
    }
}

pub const BACKUP_PREFIX: &str = "Tags.backup_";

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
}

/// Backup + scrittura atomica. Ritorna il percorso del backup.
pub fn write_atomic(tags_path: &Path, data: &[u8]) -> std::io::Result<PathBuf> {
    let folder = tags_path.parent().unwrap_or(Path::new("."));
    let mut backup = folder.join(format!("{BACKUP_PREFIX}{}", stamp()));
    let mut n = 1;
    while backup.exists() {
        backup = folder.join(format!("{BACKUP_PREFIX}{}_{n}", stamp()));
        n += 1;
    }
    fs::copy(tags_path, &backup)?;
    replace_file(tags_path, data)?;
    Ok(backup)
}

/// Scrive in un file temporaneo e poi lo mette al posto dell'originale:
/// se qualcosa va storto a meta, l'originale resta intatto.
fn replace_file(target: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = target.with_extension("tmp_ripara");
    let result = (|| {
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
        }
        // Un file in sola lettura non si puo sostituire: tolgo l'attributo e poi lo rimetto.
        let readonly = fs::metadata(target).map(|m| m.permissions().readonly()).unwrap_or(false);
        if readonly {
            set_readonly(target, false)?;
        }
        // Antivirus e OneDrive a volte tengono il file aperto per un attimo: riprovo.
        let mut last = Ok(());
        for attempt in 0..6 {
            last = fs::rename(&tmp, target);
            if last.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(150 * (attempt + 1)));
        }
        if readonly {
            let _ = set_readonly(target, true);
        }
        last
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn set_readonly(path: &Path, on: bool) -> std::io::Result<()> {
    let mut perm = fs::metadata(path)?.permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(on);
    fs::set_permissions(path, perm)
}

#[derive(Clone, Debug)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub name: String,
    pub modified: Option<chrono::DateTime<chrono::Local>>,
    pub favorites: usize,
}

pub fn list_backups(tags_path: &Path) -> Vec<BackupInfo> {
    let Some(folder) = tags_path.parent() else { return vec![] };
    let Ok(rd) = fs::read_dir(folder) else { return vec![] };
    let mut out: Vec<BackupInfo> = rd
        .flatten()
        .filter_map(|d| {
            let name = d.file_name().to_string_lossy().into_owned();
            if !name.starts_with(BACKUP_PREFIX) {
                return None;
            }
            let path = d.path();
            let modified = d.metadata().ok().and_then(|m| m.modified().ok()).map(chrono::DateTime::from);
            let favorites = TagsFile::read(&path).map(|t| t.entries.len()).unwrap_or(0);
            Some(BackupInfo { path, name, modified, favorites })
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Ripristina un backup. Il file attuale viene salvato prima come nuovo backup.
pub fn restore_backup(tags_path: &Path, backup: &Path) -> std::io::Result<PathBuf> {
    let folder = tags_path.parent().unwrap_or(Path::new("."));
    let safety = folder.join(format!("{BACKUP_PREFIX}{}_before_restore", stamp()));
    fs::copy(tags_path, &safety)?;
    let data = fs::read(backup)?;
    replace_file(tags_path, &data)?;
    Ok(safety)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_identico() {
        let src = b"@TagCase=vst3,*,plugin,vst\r\n\"c:\\a b\\x.wav\",*\r\nc:\\a\\y.wav,*\r\n";
        let t = TagsFile::parse("Tags".into(), src);
        assert_eq!(t.entries.len(), 2);
        assert_eq!(t.entries[0].path, "c:\\a b\\x.wav");
        assert_eq!(t.entries[1].path, "c:\\a\\y.wav");
        let edits = vec![Edit::Keep; 2];
        assert_eq!(t.build(&edits), src.to_vec());
    }

    #[test]
    fn modifiche() {
        let src = b"@H\r\n\"c:\\a b\\x.wav\",*\r\nc:\\a\\y.wav,*\r\n";
        let t = TagsFile::parse("Tags".into(), src);
        assert!(t.lowercase_paths);
        let out = t.build(&[Edit::Replace("D:\\New Dir\\X.wav".into()), Edit::Remove]);
        assert_eq!(out, b"@H\r\n\"d:\\new dir\\x.wav\",*\r\n".to_vec());
    }

    #[test]
    fn scrive_anche_se_in_sola_lettura() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Tags");
        fs::write(&p, b"a,*\r\n").unwrap();
        set_readonly(&p, true).unwrap();
        let b = write_atomic(&p, b"b,*\r\n").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"b,*\r\n");
        assert_eq!(fs::read(&b).unwrap(), b"a,*\r\n");
        assert!(fs::metadata(&p).unwrap().permissions().readonly());
        assert!(!p.with_extension("tmp_ripara").exists());
    }

    #[test]
    fn file_reale_identico() {
        let p = std::env::var("TAGS_REALE").unwrap_or_default();
        if p.is_empty() {
            return;
        }
        let data = fs::read(&p).unwrap();
        let t = TagsFile::parse(p.into(), &data);
        assert!(t.entries.len() > 400);
        assert_eq!(t.build(&vec![Edit::Keep; t.entries.len()]), data);
    }
}
