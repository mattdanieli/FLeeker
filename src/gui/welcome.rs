//! Finestra di benvenuto: cinque schede che spiegano a cosa serve FLeeker e come si usa.
//! Si apre da sola al primo avvio e poi dal pulsante "?" in alto.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Key, Margin, Painter, Pos2, Rect, RichText,
    Sense, Shape, Stroke, TextureId, Vec2,
};

use super::{App, Lang, Palette, accent_button, bold, tint};

const PAGES: usize = 5;
const WIDTH: f32 = 760.0;
const ART_H: f32 = 300.0;

pub struct Welcome {
    page: usize,
    /// Quando e comparsa la scheda attuale (per le animazioni).
    since: Option<f64>,
    /// +1 avanti, -1 indietro: da che lato entra la scheda.
    dir: f32,
}

impl Welcome {
    pub fn new() -> Self {
        // Solo per gli screenshot di sviluppo: apre direttamente una scheda.
        #[cfg(debug_assertions)]
        if let Some(n) = std::env::var("FLEEKER_WELCOME_PAGE").ok().and_then(|v| v.parse().ok()) {
            return Welcome { page: n, since: None, dir: 1.0 };
        }
        Welcome { page: 0, since: None, dir: 1.0 }
    }
}

impl App {
    pub(super) fn welcome_ui(&mut self, ctx: &egui::Context) {
        let Some(w) = self.welcome.as_mut() else { return };
        let p = self.pal;
        let lang = self.lang;
        let logo = self.logo.id();
        let now = ctx.input(|i| i.time);
        let t = (now - *w.since.get_or_insert(now)) as f32;
        if t < 3.0 {
            ctx.request_repaint();
        }
        let (page, dir) = (w.page, w.dir);
        let last = page == PAGES - 1;

        let frame = Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(18))
            .inner_margin(Margin::ZERO);
        let mut go: Option<usize> = None;
        let mut close = false;

        let r = egui::Modal::new(egui::Id::unique("welcome")).frame(frame).show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let e = ease(t / 0.45);

            // Illustrazione
            let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH, ART_H), Sense::hover());
            let bg = if self.settings.dark { Color32::from_rgb(0x17, 0x1a, 0x20) } else { p.card_hi };
            ui.painter().rect_filled(rect, CornerRadius { nw: 18, ne: 18, sw: 0, se: 0 }, bg);
            let mut painter = ui.painter_at(rect);
            painter.multiply_opacity(e);
            let art = rect.translate(Vec2::new((1.0 - e) * 36.0 * dir, 0.0));
            match page {
                0 => art_hello(&painter, art, &p, t, logo),
                1 => art_lost(&painter, art, &p, t),
                2 => art_found(&painter, art, &p, t),
                3 => art_steps(&painter, art, &p, lang, t),
                _ => art_safe(&painter, art, &p, lang, t),
            }

            // Testo
            Frame::new().inner_margin(Margin { left: 40, right: 40, top: 26, bottom: 6 }).show(ui, |ui| {
                ui.set_width(WIDTH - 80.0);
                ui.set_min_height(150.0);
                ui.multiply_opacity(e);
                let (title, body) = page_text(lang, page);
                ui.label(RichText::new(format!("{} / {PAGES}", page + 1)).color(p.accent).font(bold(12.5)));
                ui.add_space(6.0);
                ui.label(RichText::new(title).font(bold(24.0)).color(p.text));
                ui.add_space(10.0);
                ui.label(RichText::new(body).size(14.5).color(p.weak).line_height(Some(21.0)));
            });

            // Piede: salta, pallini, indietro / avanti
            Frame::new().inner_margin(Margin { left: 28, right: 28, top: 8, bottom: 22 }).show(ui, |ui| {
                let (row, _) = ui.allocate_exact_size(Vec2::new(WIDTH - 56.0, 38.0), Sense::hover());
                if !last {
                    let skip = egui::Button::new(RichText::new(lang.t("Skip", "Salta")).color(p.weak).size(14.0)).frame(false);
                    if ui.put(Rect::from_min_size(row.left_top(), Vec2::new(90.0, 38.0)), skip).clicked() {
                        close = true;
                    }
                }
                let gap = 22.0;
                let x0 = row.center().x - gap * (PAGES as f32 - 1.0) / 2.0;
                for i in 0..PAGES {
                    let c = Pos2::new(x0 + gap * i as f32, row.center().y);
                    let hit = Rect::from_center_size(c, Vec2::splat(16.0));
                    let resp = ui.interact(hit, ui.scope_id().with(("dot", i)), Sense::click());
                    if resp.clicked() {
                        go = Some(i);
                    }
                    let (size, col) = if i == page {
                        (Vec2::new(24.0, 8.0), p.accent)
                    } else if resp.hovered() {
                        (Vec2::splat(8.0), p.weak)
                    } else {
                        (Vec2::splat(8.0), tint(p.weak, 90))
                    };
                    ui.painter().rect_filled(Rect::from_center_size(c, size), CornerRadius::same(4), col);
                }
                let next_label = if last { lang.t("Get started", "Inizia") } else { lang.t("Next", "Avanti") };
                let next_rect = Rect::from_min_size(Pos2::new(row.right() - 160.0, row.top()), Vec2::new(160.0, 38.0));
                if ui.put(next_rect, accent_button(&p, next_label, true)).clicked() {
                    if last {
                        close = true;
                    } else {
                        go = Some(page + 1);
                    }
                }
                if page > 0 {
                    let back = egui::Button::new(RichText::new(lang.t("Back", "Indietro")).size(14.0));
                    let back_rect = Rect::from_min_size(Pos2::new(next_rect.left() - 118.0, row.top()), Vec2::new(108.0, 38.0));
                    if ui.put(back_rect, back).clicked() {
                        go = Some(page - 1);
                    }
                }
            });
        });

        ctx.input(|i| {
            if i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::Enter) {
                if last && i.key_pressed(Key::Enter) {
                    close = true;
                } else if !last {
                    go = Some(page + 1);
                }
            }
            if i.key_pressed(Key::ArrowLeft) && page > 0 {
                go = Some(page - 1);
            }
        });
        if r.should_close() {
            close = true;
        }

        if close {
            self.welcome = None;
            self.settings.welcome_seen = true;
        } else if let Some(n) = go.filter(|&n| n != page)
            && let Some(w) = self.welcome.as_mut()
        {
            w.dir = if n > page { 1.0 } else { -1.0 };
            w.page = n;
            w.since = None;
        }
    }
}

fn page_text(l: Lang, page: usize) -> (&'static str, &'static str) {
    match page {
        0 => (
            l.t("Welcome to FLeeker", "Benvenuto in FLeeker"),
            l.t(
                "FLeeker brings back the favorites you starred in the FL Studio browser after their samples were moved. Here is a quick tour: it takes 30 seconds.",
                "FLeeker ti restituisce i preferiti che hai segnato con la stella nel browser di FL Studio dopo che i loro sample sono stati spostati. Ecco un giro veloce: ci vogliono 30 secondi.",
            ),
        ),
        1 => (
            l.t("Lost your favorites?", "Hai perso i tuoi preferiti?"),
            l.t(
                "Move a sample folder, rename a pack or switch to a new drive, and FL Studio can no longer find the samples you starred. Your Starred list fills up with dead entries.",
                "Sposti una cartella di sample, rinomini un pack o passi a un nuovo disco, e FL Studio non trova più i sample che avevi segnato con la stella. La lista dei preferiti si riempie di voci morte.",
            ),
        ),
        2 => (
            l.t("FLeeker finds them again", "FLeeker li ritrova"),
            l.t(
                "It searches the folders you choose for every missing sample and relinks each favorite to its new place. It recognizes a sample by its content too, so it finds it even if it was renamed.",
                "Cerca ogni sample mancante nelle cartelle che scegli e ricollega ogni preferito al suo nuovo posto. Riconosce un sample anche dal contenuto, quindi lo trova anche se è stato rinominato.",
            ),
        ),
        3 => (
            l.t("Three steps", "Tre passaggi"),
            l.t(
                "FLeeker finds the FL Studio favorites file by itself. Add the folders where your samples are now, press \"Scan for missing samples\", check the result, then apply the changes.",
                "FLeeker trova da solo il file dei preferiti di FL Studio. Aggiungi le cartelle dove si trovano ora i tuoi sample, premi \"Cerca i sample mancanti\", controlla il risultato e applica le modifiche.",
            ),
        ),
        _ => (
            l.t("Safe by design", "Sicuro per come è fatto"),
            l.t(
                "FLeeker starts in Preview mode: nothing is written until you switch to Live mode. A backup is saved before every change, and nothing is written while FL Studio is open. Ready? Let's go.",
                "FLeeker parte in modalità anteprima: non scrive niente finché non passi alla modalità reale. Prima di ogni modifica salva un backup, e non scrive niente mentre FL Studio è aperto. Pronto? Si parte.",
            ),
        ),
    }
}

const SAMPLES: [&str; 6] = ["Kick Punchy.wav", "Snare Crisp.wav", "808 Deep.wav", "Hat Loop 01.wav", "Vox Chop.wav", "Pad Warm.wav"];

/// Pannello come il browser di FL Studio, con le stelle.
fn browser(pt: &Painter, r: Rect, p: &Palette) -> (Rect, f32) {
    pt.rect_filled(r, CornerRadius::same(12), p.card);
    pt.rect_stroke(r, CornerRadius::same(12), Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
    pt.text(r.left_top() + Vec2::new(16.0, 20.0), Align2::LEFT_CENTER, "★  Starred", super::bold(13.5), p.yellow);
    let body = Rect::from_min_max(r.left_top() + Vec2::new(8.0, 38.0), r.right_bottom() - Vec2::new(8.0, 8.0));
    (body, body.height() / SAMPLES.len() as f32)
}

/// Alcuni preferiti diventano voci morte.
fn art_lost(pt: &Painter, r: Rect, p: &Palette, t: f32) {
    let panel = Rect::from_center_size(r.center(), Vec2::new(380.0, 252.0));
    let (body, row_h) = browser(pt, panel, p);
    for (i, name) in SAMPLES.iter().enumerate() {
        let y = body.top() + i as f32 * row_h;
        let row = Rect::from_min_size(Pos2::new(body.left(), y), Vec2::new(body.width(), row_h));
        if i % 2 == 1 {
            pt.rect_filled(row, CornerRadius::same(6), tint(p.card_hi, 160));
        }
        let broken = [1usize, 3, 4].iter().position(|&b| b == i);
        let k = broken.map_or(0.0, |n| phase(t, 0.6 + n as f32 * 0.35, 0.4));
        let star = mix(p.yellow, p.gray, k);
        let text = mix(p.text, p.gray, k);
        pt.text(Pos2::new(row.left() + 12.0, row.center().y), Align2::LEFT_CENTER, "★", FontId::proportional(14.0), star);
        let g = pt.text(Pos2::new(row.left() + 36.0, row.center().y), Align2::LEFT_CENTER, *name, FontId::proportional(13.5), text);
        if k > 0.0 {
            let mid = g.center().y;
            pt.line_segment([Pos2::new(g.left(), mid), Pos2::new(lerp(g.left(), g.right(), k), mid)], Stroke::new(1.5, tint(p.red, 200)));
            let c = Pos2::new(row.right() - 18.0, row.center().y);
            pt.circle_filled(c, 9.0 * k, tint(p.red, (255.0 * k) as u8));
            pt.text(c, Align2::CENTER_CENTER, "✖", FontId::proportional(10.0), tint(Color32::WHITE, (255.0 * k) as u8));
        }
    }
}

/// La lente passa sui preferiti mancanti e li ricollega al nuovo percorso.
fn art_found(pt: &Painter, r: Rect, p: &Palette, t: f32) {
    let names = [SAMPLES[1], SAMPLES[3], SAMPLES[4]];
    let found = ["D:/Samples/Drums/Snare Crisp.wav", "D:/Samples/Loops/Hat Loop 01.wav", "D:/Vocals/Vox Chop (edit).wav"];
    let left = Rect::from_min_size(r.left_top() + Vec2::new(40.0, 52.0), Vec2::new(230.0, 196.0));
    let right = Rect::from_min_size(Pos2::new(r.right() - 330.0, r.top() + 52.0), Vec2::new(290.0, 196.0));
    for (rect, title) in [(left, "★  Starred"), (right, "D:/Samples")] {
        pt.rect_filled(rect, CornerRadius::same(12), p.card);
        pt.rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        pt.text(rect.left_top() + Vec2::new(16.0, 20.0), Align2::LEFT_CENTER, title, super::bold(13.0), p.weak);
    }
    let row_h = 46.0;
    for i in 0..3 {
        let k = phase(t, 0.5 + i as f32 * 0.45, 0.5);
        let ly = left.top() + 58.0 + i as f32 * row_h;
        let ry = right.top() + 58.0 + i as f32 * row_h;
        let col = mix(p.red, p.green, k);
        // a sinistra: il preferito
        pt.text(Pos2::new(left.left() + 16.0, ly), Align2::LEFT_CENTER, if k > 0.5 { "✔" } else { "✖" }, FontId::proportional(12.0), col);
        pt.text(Pos2::new(left.left() + 36.0, ly), Align2::LEFT_CENTER, names[i], FontId::proportional(13.0), mix(p.gray, p.text, k));
        // a destra: dove si trova adesso
        let a = (255.0 * k) as u8;
        pt.with_clip_rect(right.shrink(8.0)).text(
            Pos2::new(right.left() + 16.0, ry),
            Align2::LEFT_CENTER,
            found[i],
            FontId::proportional(12.0),
            tint(p.text, a),
        );
        // il collegamento
        let from = Pos2::new(left.right() + 6.0, ly);
        let to = Pos2::new(right.left() - 6.0, ry);
        if k > 0.0 {
            let end = from + (to - from) * k;
            pt.line_segment([from, end], Stroke::new(2.0, tint(p.green, a)));
            pt.circle_filled(from, 3.5, tint(p.green, a));
            pt.circle_filled(end, 3.5, tint(p.green, a));
        }
    }
    // lente che scende lungo l'elenco
    let k = phase(t, 0.3, 1.6);
    let c = Pos2::new(r.center().x, lerp(left.top() + 50.0, left.top() + 58.0 + 2.0 * row_h, k));
    lens(pt, c, 26.0, (255.0 * (1.0 - phase(t, 2.0, 0.5))) as u8);
}

fn lens(pt: &Painter, c: Pos2, rad: f32, a: u8) {
    let ring = tint(Color32::from_rgb(0xee, 0xf0, 0xf5), a);
    pt.circle_filled(c, rad, tint(Color32::WHITE, (a as f32 * 0.12) as u8));
    pt.circle_stroke(c, rad, Stroke::new(5.0, ring));
    let d = Vec2::new(1.0, 1.0).normalized();
    pt.line_segment([c + d * (rad + 3.0), c + d * (rad + 18.0)], Stroke::new(8.0, ring));
}

/// Tre schede: file dei preferiti, cartelle, applica.
fn art_steps(pt: &Painter, r: Rect, p: &Palette, l: Lang, t: f32) {
    let labels = [
        l.t("Favorites file", "File dei preferiti"),
        l.t("Scan locations", "Posizioni di scansione"),
        l.t("✔  Apply changes", "✔  Applica modifiche"),
    ];
    let cw = 196.0;
    let ch = 210.0;
    let gap = 28.0;
    let x0 = r.center().x - (3.0 * cw + 2.0 * gap) / 2.0;
    for (i, label) in labels.iter().enumerate() {
        let k = phase(t, 0.15 + i as f32 * 0.22, 0.55);
        let card = Rect::from_min_size(
            Pos2::new(x0 + i as f32 * (cw + gap), r.center().y - ch / 2.0 + (1.0 - k) * 24.0),
            Vec2::new(cw, ch),
        );
        let a = (255.0 * k) as u8;
        pt.rect_filled(card, CornerRadius::same(14), tint(p.card, a));
        pt.rect_stroke(card, CornerRadius::same(14), Stroke::new(1.0, tint(p.border, a)), egui::StrokeKind::Inside);
        let badge = Pos2::new(card.left() + 26.0, card.top() + 26.0);
        pt.circle_filled(badge, 13.0, tint(LANES[i], a));
        pt.text(badge, Align2::CENTER_CENTER, format!("{}", i + 1), super::bold(13.0), tint(Color32::from_rgb(0x1a, 0x10, 0x05), a));
        let c = Pos2::new(card.center().x, card.top() + 98.0);
        match i {
            0 => {
                let doc = Rect::from_center_size(c, Vec2::new(58.0, 72.0));
                pt.rect_filled(doc, CornerRadius::same(8), tint(p.card_hi, a));
                pt.rect_stroke(doc, CornerRadius::same(8), Stroke::new(2.0, tint(p.yellow, a)), egui::StrokeKind::Inside);
                pt.text(doc.center() - Vec2::new(0.0, 8.0), Align2::CENTER_CENTER, "★", FontId::proportional(24.0), tint(p.yellow, a));
                pt.text(doc.center() + Vec2::new(0.0, 20.0), Align2::CENTER_CENTER, "Tags", super::bold(11.0), tint(p.weak, a));
            }
            1 => {
                for j in 0..3 {
                    let o = Vec2::new(-18.0 + j as f32 * 18.0, -14.0 + j as f32 * 14.0);
                    folder(pt, c + o, tint(LANES[j + 1], a));
                }
            }
            _ => {
                let pulse = 1.0 + 0.06 * (t * 3.0).sin().max(0.0) * (1.0 - phase(t, 2.2, 0.6));
                pt.circle_filled(c, 34.0 * pulse, tint(p.green, (60.0 * k) as u8));
                pt.circle_stroke(c, 34.0 * pulse, Stroke::new(3.0, tint(p.green, a)));
                pt.line_segment([c + Vec2::new(-14.0, 1.0), c + Vec2::new(-4.0, 11.0)], Stroke::new(5.0, tint(p.green, a)));
                pt.line_segment([c + Vec2::new(-4.0, 11.0), c + Vec2::new(16.0, -11.0)], Stroke::new(5.0, tint(p.green, a)));
            }
        }
        let g = pt.layout(label.trim().to_string(), super::bold(13.5), tint(p.text, a), cw - 24.0);
        pt.galley(Pos2::new(card.center().x - g.size().x / 2.0, card.bottom() - 30.0 - g.size().y / 2.0), g, tint(p.text, a));
    }
}

fn folder(pt: &Painter, c: Pos2, col: Color32) {
    let body = Rect::from_center_size(c + Vec2::new(0.0, 4.0), Vec2::new(64.0, 44.0));
    let tab = Rect::from_min_size(body.left_top() - Vec2::new(0.0, 8.0), Vec2::new(26.0, 12.0));
    pt.rect_filled(tab, CornerRadius::same(4), col);
    pt.rect_filled(body, CornerRadius::same(6), col);
    pt.rect_stroke(body, CornerRadius::same(6), Stroke::new(1.5, Color32::from_black_alpha(60)), egui::StrokeKind::Inside);
}

/// Anteprima, backup, FL Studio chiuso.
fn art_safe(pt: &Painter, r: Rect, p: &Palette, l: Lang, t: f32) {
    let rows = [
        (l.t("Preview mode", "Modalità anteprima"), p.blue),
        (l.t("Backup before every change", "Backup prima di ogni modifica"), p.purple),
        (l.t("FL Studio is closed", "FL Studio è chiuso"), p.green),
    ];
    let w = 400.0;
    let h = 60.0;
    let gap = 16.0;
    let top = r.center().y - (3.0 * h + 2.0 * gap) / 2.0;
    for (i, (label, col)) in rows.iter().enumerate() {
        let k = phase(t, 0.2 + i as f32 * 0.3, 0.5);
        let a = (255.0 * k) as u8;
        let rr = Rect::from_min_size(Pos2::new(r.center().x - w / 2.0 + (1.0 - k) * 30.0, top + i as f32 * (h + gap)), Vec2::new(w, h));
        pt.rect_filled(rr, CornerRadius::same(14), tint(p.card, a));
        pt.rect_stroke(rr, CornerRadius::same(14), Stroke::new(1.5, tint(*col, (a as f32 * 0.6) as u8)), egui::StrokeKind::Inside);
        let c = Pos2::new(rr.left() + 34.0, rr.center().y);
        pt.circle_filled(c, 17.0, tint(*col, a));
        match i {
            0 => {
                // occhio
                let pts: Vec<Pos2> = (0..=16)
                    .map(|j| {
                        let x = -9.0 + 18.0 * j as f32 / 16.0;
                        c + Vec2::new(x, -5.0 * (1.0 - (x / 9.0).powi(2)))
                    })
                    .chain((0..=16).map(|j| {
                        let x = 9.0 - 18.0 * j as f32 / 16.0;
                        c + Vec2::new(x, 5.0 * (1.0 - (x / 9.0).powi(2)))
                    }))
                    .collect();
                pt.add(Shape::closed_line(pts, Stroke::new(2.0, tint(Color32::WHITE, a))));
                pt.circle_filled(c, 2.6, tint(Color32::WHITE, a));
            }
            1 => {
                for (o, al) in [(Vec2::new(3.0, -3.0), 0.55), (Vec2::new(-2.0, 2.0), 1.0)] {
                    let d = Rect::from_center_size(c + o, Vec2::new(11.0, 14.0));
                    pt.rect_filled(d, CornerRadius::same(2), tint(Color32::WHITE, (a as f32 * al) as u8));
                }
            }
            _ => {
                pt.line_segment([c + Vec2::new(-7.0, 0.0), c + Vec2::new(-2.0, 6.0)], Stroke::new(3.0, tint(Color32::WHITE, a)));
                pt.line_segment([c + Vec2::new(-2.0, 6.0), c + Vec2::new(8.0, -6.0)], Stroke::new(3.0, tint(Color32::WHITE, a)));
            }
        }
        pt.text(Pos2::new(rr.left() + 66.0, rr.center().y), Align2::LEFT_CENTER, *label, super::bold(15.0), tint(p.text, a));
    }
}

// --------------------------------------------------------------------------
// Illustrazioni (disegnate, niente immagini: restano nitide a ogni scala)
// --------------------------------------------------------------------------

const LANES: [Color32; 4] = [
    Color32::from_rgb(0xff, 0x8a, 0x2b),
    Color32::from_rgb(0xf2, 0xc1, 0x4e),
    Color32::from_rgb(0x3e, 0xcf, 0x8e),
    Color32::from_rgb(0x5a, 0xaa, 0xff),
];

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x).powi(3)
}

/// Avanzamento 0..1 di un'animazione che parte a `start` secondi e dura `dur`.
fn phase(t: f32, start: f32, dur: f32) -> f32 {
    ease((t - start) / dur)
}

fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x
}

fn art_hello(pt: &Painter, r: Rect, p: &Palette, t: f32, logo: TextureId) {
    let c = r.center();
    // corsie di clip che entrano dai lati
    let h = 26.0;
    let gap = 14.0;
    let top = c.y - (4.0 * h + 3.0 * gap) / 2.0;
    let pieces: [&[(f32, f32)]; 4] = [
        &[(0.02, 0.10), (0.15, 0.08), (0.27, 0.06), (0.67, 0.09), (0.80, 0.12)],
        &[(0.05, 0.14), (0.22, 0.07), (0.70, 0.06), (0.79, 0.16)],
        &[(0.01, 0.07), (0.11, 0.12), (0.26, 0.05), (0.66, 0.13), (0.83, 0.07)],
        &[(0.04, 0.09), (0.16, 0.13), (0.69, 0.08), (0.80, 0.10), (0.93, 0.05)],
    ];
    for (i, row) in pieces.iter().enumerate() {
        let k = phase(t, 0.05 * i as f32, 0.7);
        let y = top + i as f32 * (h + gap);
        for &(x, w) in row.iter() {
            let left = x < 0.5;
            let slide = (1.0 - k) * 120.0 * if left { -1.0 } else { 1.0 };
            let x0 = r.left() + 30.0 + x * (r.width() - 60.0) + slide;
            let rr = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w * (r.width() - 60.0) - 6.0, h));
            pt.rect_filled(rr, CornerRadius::same(6), tint(LANES[i], (150.0 * k) as u8));
        }
    }
    // logo
    let s = lerp(0.82, 1.0, phase(t, 0.0, 0.6)) * 150.0;
    pt.circle_filled(c, s * 0.62, tint(Color32::BLACK, 60));
    pt.image(
        logo,
        Rect::from_center_size(c, Vec2::splat(s)),
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    let _ = p;
}

fn mix(a: Color32, b: Color32, k: f32) -> Color32 {
    let m = |x: u8, y: u8| lerp(x as f32, y as f32, k) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}
