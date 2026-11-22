//! Finestra di scelta di un file, condivisa da cruscotto e sessioni: il
//! percorso si scrive in un campo, con l'elenco dei file adatti della
//! cartella per sceglierne uno con un clic.
//!
//! `egui-file-dialog` 0.14 (la versione per egui 0.35) dipende da `sysinfo`
//! 0.39, che richiede Rust 1.95: con la versione di Rust del progetto (1.93)
//! non compila, quindi qui si usa un campo di testo.

use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText, ScrollArea};

use super::tema;

/// Esito di un fotogramma della finestra.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum EsitoScelta {
    /// Ancora aperta.
    Aperta,
    /// L'utente ha confermato questo percorso.
    Conferma(PathBuf),
    Annulla,
}

/// Finestra modale con campo del percorso ed elenco dei file.
#[derive(Clone, Debug)]
pub(crate) struct SceltaFile {
    pub titolo: String,
    pub spiegazione: String,
    /// Testo del pulsante di conferma («Importa», «Esporta»…).
    pub pulsante: String,
    pub percorso: String,
    /// Finali dei nomi da elencare, in minuscolo (per esempio `.json`).
    pub finali: Vec<&'static str>,
    pub suggerimento: String,
}

impl SceltaFile {
    /// Finestra che propone `cartella/nome`.
    pub fn new(titolo: &str, pulsante: &str, cartella: &Path, nome: &str) -> Self {
        SceltaFile {
            titolo: titolo.into(),
            spiegazione: String::new(),
            pulsante: pulsante.into(),
            percorso: cartella.join(nome).display().to_string(),
            finali: Vec::new(),
            suggerimento: format!(r"C:\Users\…\Documents\{nome}"),
        }
    }

    pub fn spiegazione(mut self, testo: &str) -> Self {
        self.spiegazione = testo.into();
        self
    }

    pub fn finali(mut self, finali: &[&'static str]) -> Self {
        self.finali = finali.to_vec();
        self
    }

    /// Cartella di cui mostrare i file: quella del percorso scritto, se esiste.
    fn cartella(&self) -> PathBuf {
        let p = PathBuf::from(self.percorso.trim());
        if p.is_dir() {
            p
        } else {
            p.parent()
                .map(Path::to_path_buf)
                .filter(|c| c.is_dir())
                .unwrap_or_else(cartella_documenti)
        }
    }

    /// Disegna la finestra; `id` la distingue dalle altre.
    pub fn mostra(&mut self, ctx: &egui::Context, id: &str) -> EsitoScelta {
        let mut esegui = false;
        let mut chiudi = false;
        let largo = (ctx.content_rect().width() - 2.0 * tema::SP_6).clamp(280.0, 640.0);
        egui::Modal::new(egui::Id::new(("scelta_file", id))).show(ctx, |ui| {
            ui.set_width(largo);
            ui.label(RichText::new(&self.titolo).heading());
            if !self.spiegazione.is_empty() {
                ui.label(RichText::new(&self.spiegazione).color(tema::TESTO_2));
            }
            ui.add_space(tema::SP_2);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.percorso)
                    .hint_text(self.suggerimento.as_str())
                    .desired_width(ui.available_width()),
            );
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                esegui = true;
            }
            let cartella = self.cartella();
            let file = elenca_file(&cartella, &self.finali);
            ui.add_space(tema::SP_2);
            ui.label(
                RichText::new(if file.is_empty() {
                    format!("Nessun file adatto in {}", cartella.display())
                } else {
                    format!("File in {}", cartella.display())
                })
                .small()
                .color(tema::TESTO_3),
            );
            if !file.is_empty() {
                ScrollArea::vertical()
                    .id_salt(("scelta_file_elenco", id))
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for f in &file {
                            let nome = f
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            let scelto = Path::new(self.percorso.trim()) == f.as_path();
                            if ui.selectable_label(scelto, nome).clicked() {
                                self.percorso = f.display().to_string();
                            }
                        }
                    });
            }
            ui.add_space(tema::SP_3);
            ui.horizontal(|ui| {
                let pronto = !self.percorso.trim().is_empty();
                if ui
                    .add_enabled(
                        pronto,
                        egui::Button::new(
                            RichText::new(self.pulsante.as_str())
                                .color(tema::TESTO_SU_COLORE)
                                .strong(),
                        )
                        .fill(tema::ACCENTO),
                    )
                    .clicked()
                {
                    esegui = true;
                }
                if ui.button("Annulla").clicked() {
                    chiudi = true;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            chiudi = true;
        }
        if esegui && !self.percorso.trim().is_empty() {
            EsitoScelta::Conferma(PathBuf::from(self.percorso.trim()))
        } else if chiudi {
            EsitoScelta::Annulla
        } else {
            EsitoScelta::Aperta
        }
    }
}

/// Cartella proposta per importare ed esportare: i Documenti dell'utente.
pub(crate) fn cartella_documenti() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|u| PathBuf::from(u).join("Documents"))
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// File di una cartella il cui nome finisce con uno dei `finali` (tutti se
/// l'elenco è vuoto), in ordine alfabetico, al massimo 50.
pub(crate) fn elenca_file(cartella: &Path, finali: &[&str]) -> Vec<PathBuf> {
    let Ok(voci) = std::fs::read_dir(cartella) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = voci
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let nome = p
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            p.is_file() && (finali.is_empty() || finali.iter().any(|f| nome.ends_with(f)))
        })
        .collect();
    v.sort();
    v.truncate(50);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elenco_filtrato_per_finale() {
        let dir = std::env::temp_dir().join(format!("obdio_scelta_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sotto.json")).unwrap();
        for n in ["b.JSON", "a.json", "c.csv", "d.obdio.jsonl", "e.txt"] {
            std::fs::write(dir.join(n), "x").unwrap();
        }
        let nomi = |f: &[&str]| -> Vec<String> {
            elenca_file(&dir, f)
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect()
        };
        assert_eq!(nomi(&[".json"]), vec!["a.json", "b.JSON"]);
        assert_eq!(
            nomi(&[".obdio.jsonl", ".csv"]),
            vec!["c.csv", "d.obdio.jsonl"]
        );
        assert_eq!(nomi(&[]).len(), 5, "le cartelle non compaiono");
        assert!(elenca_file(&dir.join("manca"), &[]).is_empty());
        let s = SceltaFile::new("T", "Esporta", &dir, "x.csv");
        assert_eq!(s.cartella(), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
