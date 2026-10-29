//! Cose legate al sistema: FL Studio aperto, dove sta il file Tags, Esplora file / Finder.

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
fn hidden(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(windows))]
fn hidden(cmd: &mut Command) -> &mut Command {
    cmd
}

/// True se FL Studio e in esecuzione (Windows e macOS).
pub fn fl_studio_running() -> bool {
    if cfg!(windows) {
        let Ok(out) = hidden(Command::new("tasklist").args(["/FO", "CSV", "/NH"])).output() else {
            return false;
        };
        let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
        ["\"fl64.exe\"", "\"fl.exe\"", "\"fl studio"].iter().any(|n| text.contains(n))
    } else if cfg!(target_os = "macos") {
        let Ok(out) = Command::new("ps").args(["-axo", "comm="]).output() else {
            return false;
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.to_lowercase().contains("/fl studio") && l.contains("/Contents/MacOS/"))
    } else {
        false
    }
}

/// Mostra il file in Esplora file / Finder (o la cartella, se il file non c'e).
pub fn reveal(path: &str) {
    let p = Path::new(path);
    if cfg!(windows) {
        if p.exists() {
            let _ = hidden(&mut Command::new("explorer")).arg(format!("/select,{path}")).spawn();
        } else if let Some(parent) = p.parent().filter(|d| d.exists()) {
            let _ = hidden(&mut Command::new("explorer")).arg(parent).spawn();
        }
    } else if cfg!(target_os = "macos") {
        if p.exists() {
            let _ = Command::new("open").arg("-R").arg(p).spawn();
        } else if let Some(parent) = p.parent().filter(|d| d.exists()) {
            let _ = Command::new("open").arg(parent).spawn();
        }
    } else if let Some(parent) = p.parent() {
        let _ = Command::new("xdg-open").arg(parent).spawn();
    }
}

pub fn drive_exists(d: &str) -> bool {
    Path::new(d).exists()
}

/// Cerca il file Tags: cartelle Documenti (anche OneDrive), cartelle indicate nel
/// registro di FL Studio (cartella dati spostata) e cartelle Image-Line / FL Studio
/// nella radice di ogni disco. Prima il piu recente.
pub fn find_tags_candidates() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(d) = dirs::document_dir() {
        roots.push(d.join("Image-Line"));
    }
    if let Some(h) = dirs::home_dir() {
        for sub in ["Documents", "Documenti", "OneDrive/Documents", "OneDrive/Documenti"] {
            roots.push(h.join(sub).join("Image-Line"));
        }
        // macOS: alcune versioni tengono i dati utente anche qui
        roots.push(h.join("Library").join("Application Support").join("Image-Line"));
    }
    // Le cartelle Documenti\Image-Line sono piccole: si esplorano a fondo.
    // Quelle prese dal registro o dai dischi possono contenere librerie di sample
    // enormi, quindi si guardano solo i primi livelli, con un tetto di cartelle.
    let extra: Vec<PathBuf> = registry_dirs().into_iter().chain(drive_root_dirs()).collect();

    let mut found: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut budget = usize::MAX;
    for root in roots {
        if root.is_dir() && seen.insert(root.to_string_lossy().to_lowercase()) {
            walk_for_tags(&root, 7, &mut budget, &mut found);
        }
    }
    let mut budget = 20_000;
    for root in extra {
        if root.is_dir() && seen.insert(root.to_string_lossy().to_lowercase()) {
            walk_for_tags(&root, 4, &mut budget, &mut found);
        }
    }
    found.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    found.dedup_by(|a, b| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()));
    found
}

/// Cartelle scritte da FL Studio nel registro (HKCU\Software\Image-Line): se l'utente
/// ha spostato la cartella dati, il percorso nuovo sta qui.
fn registry_dirs() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let Ok(out) = hidden(Command::new("reg").args(["query", r"HKCU\Software\Image-Line", "/s"])).output() else {
        return Vec::new();
    };
    parse_registry_dirs(&String::from_utf8_lossy(&out.stdout))
}

fn parse_registry_dirs(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in text.lines() {
        let Some(pos) = line.find("REG_SZ").or_else(|| line.find("REG_EXPAND_SZ")) else { continue };
        let value = line[pos..].split_once(char::is_whitespace).map(|(_, v)| v.trim()).unwrap_or("");
        let value = value.trim_matches('"');
        let b = value.as_bytes();
        if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
            continue;
        }
        let lower = value.to_lowercase();
        // La cartella del programma non contiene mai il file Tags
        if lower.contains("\\program files") || lower.ends_with(".exe") || lower.ends_with(".dll") {
            continue;
        }
        // Se e un file (ha un'estensione nell'ultimo pezzo) si prende la sua cartella.
        // Si lavora sul testo perche il percorso usa sempre "\\", anche nei test su Linux.
        let value = value.trim_end_matches('\\');
        let last = value.rsplit('\\').next().unwrap_or("");
        let dir = if last.contains('.') { value.rsplit_once('\\').map(|(d, _)| d).unwrap_or(value) } else { value };
        let d = PathBuf::from(if dir.len() == 2 { format!("{dir}\\") } else { dir.to_string() });
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// Cartelle "Image-Line", "FL Studio" o "FL_User_Data" nei primi due livelli di ogni disco.
fn drive_root_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !cfg!(windows) {
        return out;
    }
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if !root.is_dir() {
            continue;
        }
        collect_fl_dirs(&root, 0, &mut out);
    }
    out
}

fn collect_fl_dirs(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for d in rd.flatten() {
        let Ok(ft) = d.file_type() else { continue };
        if !ft.is_dir() || ft.is_symlink() {
            continue;
        }
        let name = d.file_name().to_string_lossy().to_lowercase();
        if matches!(name.as_str(), "windows" | "program files" | "program files (x86)" | "$recycle.bin" | "programdata") {
            continue;
        }
        if name.contains("image-line") || name.starts_with("fl studio") || name.contains("fl_user_data") {
            out.push(d.path());
        } else if depth < 1 {
            collect_fl_dirs(&d.path(), depth + 1, out);
        }
    }
}

fn walk_for_tags(dir: &Path, levels: usize, budget: &mut usize, out: &mut Vec<PathBuf>) {
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for d in rd.flatten() {
        let Ok(ft) = d.file_type() else { continue };
        let p = d.path();
        if ft.is_dir() && !ft.is_symlink() {
            if levels > 0 {
                walk_for_tags(&p, levels - 1, budget, out);
            }
        } else if ft.is_file() && d.file_name() == "Tags" {
            let parent = dir.to_string_lossy().to_lowercase().replace('/', "\\");
            if parent.ends_with("settings\\browser") {
                out.push(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legge_cartelle_dal_registro() {
        let text = "\r\nHKEY_CURRENT_USER\\Software\\Image-Line\\Shared\\Paths\r\n    \
                    User data    REG_SZ    D:\\Musica\\FL Data\r\n    \
                    Program    REG_SZ    C:\\Program Files\\Image-Line\\FL Studio 21\r\n    \
                    Last    REG_SZ    E:\\Progetti\\song.flp\r\n    \
                    Count    REG_DWORD    0x1\r\n    \
                    Env    REG_EXPAND_SZ    %USERPROFILE%\\x\r\n";
        let dirs = parse_registry_dirs(text);
        assert_eq!(dirs, vec![PathBuf::from("D:\\Musica\\FL Data"), PathBuf::from("E:\\Progetti")]);
    }
}
