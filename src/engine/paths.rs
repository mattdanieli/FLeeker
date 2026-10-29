//! Piccole utilita sui percorsi. Funzionano sia con '\' che con '/',
//! cosi i test girano anche fuori da Windows.

/// Nome del file.
pub fn basename_any(path: &str) -> &str {
    let p = path.trim_end_matches(['\\', '/']);
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

/// Cartella che contiene il file.
pub fn parent_any(path: &str) -> &str {
    let p = path.trim_end_matches(['\\', '/']);
    match p.rfind(['\\', '/']) {
        Some(i) => &p[..i],
        None => "",
    }
}

/// Chiave di confronto: minuscolo, barre uniformi. Windows non distingue
/// maiuscole e minuscole, quindi sono lo stesso file.
pub fn norm_key(path: &str) -> String {
    path.replace('/', "\\").to_lowercase().trim_end_matches('\\').to_string()
}

/// "c:\\" per "c:\\users\\...", oppure None (percorsi di rete o Linux).
pub fn drive_of(path: &str) -> Option<String> {
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        Some(format!("{}\\", &path[..2]))
    } else {
        None
    }
}

/// Quante cartelle finali hanno in comune due percorsi, partendo dal fondo.
/// Es. "c:\\old\\vec2 breakbeat\\x.wav" e "d:\\libs\\loops\\vec2 breakbeat\\x.wav" -> 1.
pub fn shared_tail_dirs(a: &str, b: &str) -> usize {
    let pa = norm_key(parent_any(a));
    let pb = norm_key(parent_any(b));
    pa.rsplit('\\').zip(pb.rsplit('\\')).take_while(|(x, y)| x == y && !x.is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base() {
        assert_eq!(basename_any("c:\\a\\b c.wav"), "b c.wav");
        assert_eq!(basename_any("/x/y.wav"), "y.wav");
        assert_eq!(parent_any("c:\\a\\b.wav"), "c:\\a");
        assert_eq!(norm_key("C:/A/B.WAV"), "c:\\a\\b.wav");
        assert_eq!(drive_of("e:\\x"), Some("e:\\".into()));
        assert_eq!(drive_of("\\\\server\\x"), None);
        assert_eq!(shared_tail_dirs("c:\\old\\pack\\x.wav", "d:\\new\\pack\\x.wav"), 1);
        assert_eq!(shared_tail_dirs("c:\\a\\x.wav", "d:\\b\\x.wav"), 0);
    }
}
