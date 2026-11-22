//! Scelta di un file da aprire o dove salvare, condivisa da cruscotto e
//! sessioni, con un'unica interfaccia ([`SceltaFile`]) e due realizzazioni
//! scelte alla compilazione da `piattaforma::DIALOGO_DI_SISTEMA`:
//!
//! - **desktop**: la finestra di sistema (`rfd`), mostrata in un thread a parte
//!   perché è bloccante; il percorso scelto torna con un canale, sondato a
//!   ogni fotogramma, quindi l'interfaccia, i dati in diretta e la
//!   registrazione non si fermano mentre la finestra è aperta;
//! - **Android** (dove `rfd` non esiste) e collaudo automatico: una finestra
//!   modale disegnata in egui, con un campo per il percorso e l'elenco dei file
//!   adatti della cartella.
//!
//! `egui-file-dialog` 0.14 (la versione per egui 0.35) dipende da `sysinfo`
//! 0.39, che richiede Rust 1.95: con la versione di Rust del progetto (1.93)
//! non compila, quindi la realizzazione interna usa un campo di testo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(not(target_os = "android"))]
use std::time::Duration;

use eframe::egui::{self, RichText, ScrollArea};

use super::componenti;
use super::navigazione::{altezza_modale, larghezza_finestra};
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

/// Che cosa si sceglie.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Modo {
    /// Un file esistente da leggere.
    Apri,
    /// Il nome di un file da scrivere.
    Salva,
}

/// Famiglia di file con la sua etichetta nel selettore dei tipi.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Filtro {
    pub nome: &'static str,
    /// Finali dei nomi, in minuscolo e col punto (per esempio `.obdio.jsonl`).
    pub finali: &'static [&'static str],
}

pub(crate) const FILTRO_SESSIONE: Filtro = Filtro {
    nome: "Sessione OBDio (*.obdio.jsonl)",
    finali: &[".obdio.jsonl"],
};
pub(crate) const FILTRO_CSV: Filtro = Filtro {
    nome: "CSV (*.csv)",
    finali: &[".csv"],
};
pub(crate) const FILTRO_JSON: Filtro = Filtro {
    nome: "JSON (*.json)",
    finali: &[".json"],
};
/// Tutti i file importabili come sessione.
pub(crate) const FILTRO_IMPORTABILI: Filtro = Filtro {
    nome: "Sessioni e CSV (*.obdio.jsonl, *.csv, *.txt)",
    finali: &[".obdio.jsonl", ".csv", ".txt"],
};

/// Scopo della scelta: per ciascuno si ricorda l'ultima cartella usata, finché
/// l'app resta aperta.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Scopo {
    ImportaSessione,
    EsportaSessione,
    RiproduciSessione,
    ImportaCruscotto,
    EsportaCruscotto,
}

/// Ultima cartella usata per ogni scopo.
#[derive(Default, Debug)]
pub(crate) struct Memoria {
    cartelle: HashMap<Scopo, PathBuf>,
}

impl Memoria {
    /// Ricorda la cartella che contiene `file`.
    pub fn ricorda(&mut self, scopo: Scopo, file: &Path) {
        if let Some(c) = file.parent().filter(|c| !c.as_os_str().is_empty()) {
            self.cartelle.insert(scopo, c.to_path_buf());
        }
    }

    /// L'ultima cartella dello scopo, se esiste ancora.
    pub fn cartella(&self, scopo: Scopo) -> Option<PathBuf> {
        self.cartelle.get(&scopo).filter(|c| c.is_dir()).cloned()
    }
}

fn memoria() -> &'static Mutex<Memoria> {
    static M: OnceLock<Mutex<Memoria>> = OnceLock::new();
    M.get_or_init(Mutex::default)
}

/// Che cosa chiedere alla finestra di sistema.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Richiesta {
    pub titolo: String,
    pub modo: Modo,
    pub cartella: PathBuf,
    /// Nome proposto (vuoto per aprire).
    pub nome: String,
    pub filtri: Vec<Filtro>,
}

/// Chi mostra la finestra di sistema: dietro a un tratto per poterla
/// sostituire nei test, dove non deve aprirsi nulla. Il metodo blocca finché
/// l'utente non risponde, quindi gira sempre in un thread a parte.
pub(crate) trait MotoreDialogo: Send + Sync {
    fn mostra(&self, richiesta: &Richiesta) -> Option<PathBuf>;
}

/// Filtri nella forma che serve alla finestra di sistema: nome ed estensioni
/// senza punto iniziale. Per aprire si aggiunge «Tutti i file» in fondo
/// (`con_tutti`); per salvare no, per non confondere l'estensione proposta.
pub(crate) fn filtri_sistema(
    modo: Modo,
    filtri: &[Filtro],
    con_tutti: bool,
) -> Vec<(String, Vec<String>)> {
    let mut v: Vec<(String, Vec<String>)> = filtri
        .iter()
        .map(|f| {
            (
                f.nome.to_string(),
                f.finali
                    .iter()
                    .map(|e| e.trim_start_matches('.').to_string())
                    .collect(),
            )
        })
        .collect();
    if modo == Modo::Apri && con_tutti {
        v.push(("Tutti i file".into(), vec!["*".into()]));
    }
    v
}

/// Finali di tutti i filtri, senza ripetizioni (per l'elenco interno).
fn finali_dei_filtri(filtri: &[Filtro]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for f in filtri {
        for e in f.finali {
            if !v.contains(e) {
                v.push(e);
            }
        }
    }
    v
}

/// Aggiunge al nome senza estensione quella del primo filtro («x» diventa
/// «x.obdio.jsonl»). Un nome che finisce già con una finale nota, o che ha
/// un'altra estensione scritta dall'utente, resta com'è.
pub(crate) fn con_estensione(percorso: &Path, filtri: &[Filtro]) -> PathBuf {
    let Some(nome) = percorso
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
    else {
        return percorso.to_path_buf();
    };
    let Some(predefinita) = filtri.iter().find_map(|f| f.finali.first()) else {
        return percorso.to_path_buf();
    };
    let nota = filtri
        .iter()
        .flat_map(|f| f.finali.iter())
        .any(|e| nome.ends_with(e));
    if nota || percorso.extension().is_some() {
        return percorso.to_path_buf();
    }
    let mut s = percorso.as_os_str().to_os_string();
    s.push(predefinita);
    PathBuf::from(s)
}

/// Mostra la finestra e rifinisce il percorso. Se per aggiungere l'estensione
/// il nome cambia e il file così ottenuto esiste già, la conferma di
/// sovrascrittura della finestra non lo riguardava: la si ripropone con il
/// nome completo (al massimo tre volte), così nulla viene sostituito di
/// nascosto.
fn risolvi(motore: &dyn MotoreDialogo, mut richiesta: Richiesta) -> Option<PathBuf> {
    for _ in 0..3 {
        let scelto = motore.mostra(&richiesta)?;
        if richiesta.modo == Modo::Apri {
            return Some(scelto);
        }
        let finale = con_estensione(&scelto, &richiesta.filtri);
        if finale == scelto || !finale.exists() {
            return Some(finale);
        }
        if let Some(c) = finale.parent() {
            richiesta.cartella = c.to_path_buf();
        }
        richiesta.nome = finale
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
    }
    None
}

/// Risposta sondata a ogni fotogramma.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Sondaggio {
    Attesa,
    Scelto(PathBuf),
    Annullato,
}

/// Libera il segno «finestra aperta» anche se il thread va in panico.
struct Rilascio(Arc<AtomicBool>);

impl Drop for Rilascio {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Una finestra di sistema aperta in un thread a parte.
#[derive(Debug)]
pub(crate) struct DialogoNativo {
    rx: Receiver<Option<PathBuf>>,
}

impl DialogoNativo {
    /// Apre la finestra in un thread. Restituisce `None` se `in_uso` dice che
    /// ce n'è già una aperta (una sola alla volta) o se il thread non parte.
    /// `sveglia` viene chiamata a risposta pronta, per ridisegnare subito.
    pub fn avvia(
        motore: Arc<dyn MotoreDialogo>,
        richiesta: Richiesta,
        in_uso: Arc<AtomicBool>,
        sveglia: Box<dyn Fn() + Send>,
    ) -> Option<Self> {
        in_uso
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        let (tx, rx) = channel();
        let rilascio = Rilascio(in_uso);
        std::thread::Builder::new()
            .name("finestra-file".into())
            .spawn(move || {
                let _rilascio = rilascio;
                let esito = risolvi(&*motore, richiesta);
                let _ = tx.send(esito);
                sveglia();
            })
            .ok()?;
        Some(DialogoNativo { rx })
    }

    pub fn sonda(&self) -> Sondaggio {
        match self.rx.try_recv() {
            Ok(Some(p)) => Sondaggio::Scelto(p),
            Ok(None) | Err(TryRecvError::Disconnected) => Sondaggio::Annullato,
            Err(TryRecvError::Empty) => Sondaggio::Attesa,
        }
    }
}

/// Segno «una finestra di sistema è aperta» (una sola alla volta).
fn in_uso_globale() -> &'static Arc<AtomicBool> {
    static IN_USO: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    IN_USO.get_or_init(Arc::default)
}

/// Una finestra di sistema è aperta: i pulsanti che ne aprirebbero un'altra
/// vanno disabilitati.
pub(crate) fn occupato() -> bool {
    in_uso_globale().load(Ordering::SeqCst)
}

#[cfg(not(target_os = "android"))]
mod sistema {
    //! Realizzazione con `rfd`.
    use super::*;

    /// Finestra di partenza con la finestra dell'app come genitore, se nota.
    static GENITORE: Mutex<Option<rfd::FileDialog>> = Mutex::new(None);

    pub fn registra(frame: &eframe::Frame) {
        let d = rfd::FileDialog::new().set_parent(frame);
        if let Ok(mut g) = GENITORE.lock() {
            *g = Some(d);
        }
    }

    fn base() -> rfd::FileDialog {
        GENITORE
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default()
    }

    pub struct MotoreRfd;

    impl MotoreDialogo for MotoreRfd {
        #[cfg(not(target_os = "macos"))]
        fn mostra(&self, r: &Richiesta) -> Option<PathBuf> {
            let mut d = base().set_title(r.titolo.as_str());
            if r.cartella.is_dir() {
                d = d.set_directory(&r.cartella);
            }
            if r.modo == Modo::Salva && !r.nome.is_empty() {
                d = d.set_file_name(r.nome.as_str());
            }
            for (nome, est) in filtri_sistema(r.modo, &r.filtri, true) {
                d = d.add_filter(nome, &est);
            }
            match r.modo {
                Modo::Apri => d.pick_file(),
                Modo::Salva => d.save_file(),
            }
        }

        /// Su macOS la finestra sincrona ferma il ciclo degli eventi
        /// dell'app; quella asincrona no (la finestra del genitore non si
        /// passa qui: `rfd` la vuole come tratto, non come valore).
        #[cfg(target_os = "macos")]
        fn mostra(&self, r: &Richiesta) -> Option<PathBuf> {
            let mut d = rfd::AsyncFileDialog::new().set_title(r.titolo.as_str());
            if r.cartella.is_dir() {
                d = d.set_directory(&r.cartella);
            }
            if r.modo == Modo::Salva && !r.nome.is_empty() {
                d = d.set_file_name(r.nome.as_str());
            }
            for (nome, est) in filtri_sistema(r.modo, &r.filtri, false) {
                d = d.add_filter(nome, &est);
            }
            let f = match r.modo {
                Modo::Apri => futures::executor::block_on(d.pick_file()),
                Modo::Salva => futures::executor::block_on(d.save_file()),
            };
            f.map(|h| h.path().to_path_buf())
        }
    }
}

/// Indica la finestra dell'app come genitore delle finestre di sistema (si
/// chiama una volta, da `App::ui`, con il `Frame` di eframe).
#[cfg(not(target_os = "android"))]
pub(crate) fn registra_finestra(frame: &eframe::Frame) {
    sistema::registra(frame);
}

/// Richiesta di scegliere un file, con la finestra di sistema o quella interna.
#[derive(Debug)]
pub(crate) struct SceltaFile {
    pub titolo: String,
    pub spiegazione: String,
    /// Testo del pulsante di conferma («Importa», «Esporta»…).
    pub pulsante: String,
    pub percorso: String,
    /// Finali dei nomi da elencare, in minuscolo (per esempio `.json`).
    pub finali: Vec<&'static str>,
    pub suggerimento: String,
    modo: Modo,
    scopo: Option<Scopo>,
    filtri: Vec<Filtro>,
    cartella_iniziale: PathBuf,
    nome: String,
    nativo: Option<DialogoNativo>,
}

impl SceltaFile {
    /// Finestra che propone `cartella/nome` (di default per aprire).
    pub fn new(titolo: &str, pulsante: &str, cartella: &Path, nome: &str) -> Self {
        SceltaFile {
            titolo: titolo.into(),
            spiegazione: String::new(),
            pulsante: pulsante.into(),
            percorso: cartella.join(nome).display().to_string(),
            finali: Vec::new(),
            suggerimento: format!(r"C:\Users\…\Documents\{nome}"),
            modo: Modo::Apri,
            scopo: None,
            filtri: Vec::new(),
            cartella_iniziale: cartella.to_path_buf(),
            nome: nome.into(),
            nativo: None,
        }
    }

    pub fn spiegazione(mut self, testo: &str) -> Self {
        self.spiegazione = testo.into();
        self
    }

    /// Si sceglie dove salvare un file, non un file esistente.
    pub fn salva(mut self) -> Self {
        self.modo = Modo::Salva;
        self
    }

    /// Tipi di file ammessi: etichette per la finestra di sistema ed elenco
    /// della finestra interna.
    pub fn filtri(mut self, filtri: &[Filtro]) -> Self {
        self.filtri = filtri.to_vec();
        self.finali = finali_dei_filtri(filtri);
        self
    }

    /// Scopo della scelta: riparte dall'ultima cartella usata per quello scopo.
    pub fn scopo(mut self, scopo: Scopo) -> Self {
        self.scopo = Some(scopo);
        let ricordata = memoria().lock().ok().and_then(|m| m.cartella(scopo));
        if let Some(c) = ricordata {
            self.percorso = c.join(&self.nome).display().to_string();
            self.cartella_iniziale = c;
        }
        self
    }

    fn richiesta(&self) -> Richiesta {
        Richiesta {
            titolo: self.titolo.clone(),
            modo: self.modo,
            cartella: self.cartella_iniziale.clone(),
            nome: self.nome.clone(),
            filtri: self.filtri.clone(),
        }
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

    /// Chiamata a ogni fotogramma finché la scelta è aperta; `id` distingue le
    /// finestre interne. Non blocca mai.
    pub fn mostra(&mut self, ctx: &egui::Context, id: &str) -> EsitoScelta {
        let esito = self.passo(ctx, id);
        if let EsitoScelta::Conferma(p) = &esito
            && let (Some(scopo), Ok(mut m)) = (self.scopo, memoria().lock())
        {
            m.ricorda(scopo, p);
        }
        esito
    }

    fn passo(&mut self, ctx: &egui::Context, id: &str) -> EsitoScelta {
        #[cfg(not(target_os = "android"))]
        if crate::piattaforma::DIALOGO_DI_SISTEMA {
            return self.passo_sistema(ctx);
        }
        match self.disegna(ctx, id) {
            EsitoScelta::Conferma(p) if self.modo == Modo::Salva => {
                EsitoScelta::Conferma(con_estensione(&p, &self.filtri))
            }
            altro => altro,
        }
    }

    #[cfg(not(target_os = "android"))]
    fn passo_sistema(&mut self, ctx: &egui::Context) -> EsitoScelta {
        if self.nativo.is_none() {
            let c = ctx.clone();
            self.nativo = DialogoNativo::avvia(
                Arc::new(sistema::MotoreRfd),
                self.richiesta(),
                in_uso_globale().clone(),
                Box::new(move || c.request_repaint()),
            );
            if self.nativo.is_none() {
                return EsitoScelta::Annulla;
            }
        }
        match self.nativo.as_ref().map(DialogoNativo::sonda) {
            Some(Sondaggio::Scelto(p)) => EsitoScelta::Conferma(p),
            Some(Sondaggio::Attesa) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                EsitoScelta::Aperta
            }
            _ => EsitoScelta::Annulla,
        }
    }

    /// Disegna la finestra interna.
    fn disegna(&mut self, ctx: &egui::Context, id: &str) -> EsitoScelta {
        let mut esegui = false;
        let mut chiudi = false;
        let largo = larghezza_finestra(ctx, 640.0);
        let stretto = tema::Disposizione::per_larghezza(ctx.content_rect().width()).stretta();
        let alto_elenco = altezza_modale(ctx, if stretto { 380.0 } else { 300.0 }).min(240.0);
        egui::Modal::new(egui::Id::new(("scelta_file", id)))
            .frame(componenti::cornice_finestra(ctx))
            .backdrop_color(egui::Color32::from_black_alpha(170))
            .show(ctx, |ui| {
                ui.set_width(largo);
                ui.label(RichText::new(&self.titolo).heading());
                if !self.spiegazione.is_empty() {
                    componenti::paragrafo(ui, &self.spiegazione);
                }
                ui.add_space(tema::SP_2);
                let r = componenti::campo_testo(
                    ui,
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
                componenti::nota(
                    ui,
                    if file.is_empty() {
                        format!("Nessun file adatto in {}", cartella.display())
                    } else {
                        format!("File in {}", cartella.display())
                    },
                );
                if !file.is_empty() {
                    ScrollArea::vertical()
                        .id_salt(("scelta_file_elenco", id))
                        .max_height(alto_elenco)
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
                let pronto = !self.percorso.trim().is_empty();
                let mut pulsanti = |ui: &mut egui::Ui| {
                    if componenti::pulsante(
                        ui,
                        stretto,
                        pronto,
                        componenti::primario(&self.pulsante),
                    )
                    .clicked()
                    {
                        esegui = true;
                    }
                    if componenti::pulsante(ui, stretto, true, egui::Button::new("Annulla"))
                        .clicked()
                    {
                        chiudi = true;
                    }
                };
                if stretto {
                    pulsanti(ui);
                } else {
                    ui.horizontal(|ui| pulsanti(ui));
                }
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
    crate::piattaforma::cartella_documenti_utente()
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
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    fn cartella_prova(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("obdio_{nome}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn elenco_filtrato_per_finale() {
        let dir = cartella_prova("scelta");
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

    #[test]
    fn estensione_aggiunta_solo_se_manca() {
        let f = [FILTRO_SESSIONE];
        let c = |p: &str, f: &[Filtro]| con_estensione(Path::new(p), f);
        assert_eq!(c("a/x", &f), PathBuf::from("a/x.obdio.jsonl"));
        assert_eq!(c("a/x.obdio.jsonl", &f), PathBuf::from("a/x.obdio.jsonl"));
        assert_eq!(c("a/X.OBDIO.JSONL", &f), PathBuf::from("a/X.OBDIO.JSONL"));
        assert_eq!(c("a/x.csv", &[FILTRO_CSV]), PathBuf::from("a/x.csv"));
        assert_eq!(c("a/x", &[FILTRO_CSV]), PathBuf::from("a/x.csv"));
        // Un'estensione diversa scritta dall'utente si rispetta.
        assert_eq!(c("a/x.txt", &[FILTRO_CSV]), PathBuf::from("a/x.txt"));
        // Senza filtri non si inventa nulla.
        assert_eq!(c("a/x", &[]), PathBuf::from("a/x"));
        // Si usa il primo filtro con una finale.
        assert_eq!(c("x", &[FILTRO_JSON, FILTRO_CSV]), PathBuf::from("x.json"));
    }

    #[test]
    fn filtri_per_la_finestra_di_sistema() {
        let v = filtri_sistema(Modo::Apri, &[FILTRO_SESSIONE, FILTRO_CSV], true);
        assert_eq!(
            v,
            vec![
                (
                    "Sessione OBDio (*.obdio.jsonl)".to_string(),
                    vec!["obdio.jsonl".to_string()]
                ),
                ("CSV (*.csv)".to_string(), vec!["csv".to_string()]),
                ("Tutti i file".to_string(), vec!["*".to_string()]),
            ]
        );
        // Salvando non c'è «Tutti i file»; senza `con_tutti` nemmeno aprendo.
        assert_eq!(filtri_sistema(Modo::Salva, &[FILTRO_JSON], true).len(), 1);
        assert_eq!(filtri_sistema(Modo::Apri, &[FILTRO_JSON], false).len(), 1);
        let multi = filtri_sistema(Modo::Apri, &[FILTRO_IMPORTABILI], false);
        assert_eq!(multi[0].1, vec!["obdio.jsonl", "csv", "txt"]);
        assert_eq!(
            finali_dei_filtri(&[FILTRO_IMPORTABILI, FILTRO_CSV]),
            vec![".obdio.jsonl", ".csv", ".txt"]
        );
    }

    #[test]
    fn cartella_ricordata_per_scopo() {
        let dir = cartella_prova("memoria");
        let mut m = Memoria::default();
        assert_eq!(m.cartella(Scopo::ImportaSessione), None);
        m.ricorda(Scopo::ImportaSessione, &dir.join("a.csv"));
        assert_eq!(m.cartella(Scopo::ImportaSessione), Some(dir.clone()));
        assert_eq!(m.cartella(Scopo::EsportaSessione), None, "uno per scopo");
        // Un percorso senza cartella non cancella il ricordo.
        m.ricorda(Scopo::ImportaSessione, Path::new("solo_nome.csv"));
        assert_eq!(m.cartella(Scopo::ImportaSessione), Some(dir.clone()));
        // Una cartella scomparsa non si propone più.
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(m.cartella(Scopo::ImportaSessione), None);
    }

    /// Finestra finta: risponde con quanto arriva dal canale, quando arriva.
    struct Finto {
        risposte: Mutex<Receiver<Option<PathBuf>>>,
        viste: Mutex<Vec<Richiesta>>,
        panico: bool,
    }

    impl MotoreDialogo for Finto {
        fn mostra(&self, r: &Richiesta) -> Option<PathBuf> {
            self.viste.lock().unwrap().push(r.clone());
            assert!(!self.panico, "finestra finta in panico");
            self.risposte.lock().unwrap().recv().ok().flatten()
        }
    }

    fn finto(panico: bool) -> (Arc<Finto>, std::sync::mpsc::Sender<Option<PathBuf>>) {
        let (tx, rx) = channel();
        let f = Arc::new(Finto {
            risposte: Mutex::new(rx),
            viste: Mutex::new(Vec::new()),
            panico,
        });
        (f, tx)
    }

    fn richiesta(modo: Modo, cartella: &Path, nome: &str, filtri: &[Filtro]) -> Richiesta {
        Richiesta {
            titolo: "T".into(),
            modo,
            cartella: cartella.to_path_buf(),
            nome: nome.into(),
            filtri: filtri.to_vec(),
        }
    }

    fn attendi(d: &DialogoNativo) -> Sondaggio {
        let t = Instant::now();
        loop {
            let s = d.sonda();
            if s != Sondaggio::Attesa || t.elapsed() > Duration::from_secs(5) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn macchina_a_stati_scelta() {
        let in_uso = Arc::new(AtomicBool::new(false));
        let svegli = Arc::new(AtomicUsize::new(0));
        let (m, tx) = finto(false);
        let s2 = svegli.clone();
        let r = richiesta(Modo::Apri, Path::new("."), "", &[FILTRO_CSV]);
        let d = DialogoNativo::avvia(
            m.clone(),
            r.clone(),
            in_uso.clone(),
            Box::new(move || {
                s2.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .expect("si apre");
        // Aperta: in attesa, e una seconda non parte.
        assert_eq!(d.sonda(), Sondaggio::Attesa);
        assert!(in_uso.load(Ordering::SeqCst));
        let (m2, _tx2) = finto(false);
        assert!(DialogoNativo::avvia(m2, r.clone(), in_uso.clone(), Box::new(|| {})).is_none());
        // Risposta: percorso scelto, finestra libera, schermo ridisegnato.
        tx.send(Some(PathBuf::from("a.csv"))).unwrap();
        assert_eq!(attendi(&d), Sondaggio::Scelto(PathBuf::from("a.csv")));
        let t = Instant::now();
        while in_uso.load(Ordering::SeqCst) && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!in_uso.load(Ordering::SeqCst));
        assert_eq!(svegli.load(Ordering::SeqCst), 1);
        assert_eq!(m.viste.lock().unwrap()[0], r);
    }

    #[test]
    fn annullata_e_panico_liberano_la_finestra() {
        let in_uso = Arc::new(AtomicBool::new(false));
        let (m, tx) = finto(false);
        let r = richiesta(Modo::Apri, Path::new("."), "", &[]);
        let d = DialogoNativo::avvia(m, r.clone(), in_uso.clone(), Box::new(|| {})).unwrap();
        tx.send(None).unwrap();
        assert_eq!(attendi(&d), Sondaggio::Annullato);

        let (p, _tx) = finto(true);
        let d = loop {
            // Il segno si libera appena il thread precedente finisce.
            if let Some(d) =
                DialogoNativo::avvia(p.clone(), r.clone(), in_uso.clone(), Box::new(|| {}))
            {
                break d;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(attendi(&d), Sondaggio::Annullato, "un panico è un annullo");
        let t = Instant::now();
        while in_uso.load(Ordering::SeqCst) && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!in_uso.load(Ordering::SeqCst));
    }

    #[test]
    fn nome_completato_che_esiste_si_riconferma() {
        let dir = cartella_prova("risolvi");
        std::fs::write(dir.join("x.csv"), "vecchio").unwrap();
        let (m, tx) = finto(false);
        // Prima l'utente scrive «x» (il file non esiste, ma x.csv sì), poi
        // conferma il nome completo.
        tx.send(Some(dir.join("x"))).unwrap();
        tx.send(Some(dir.join("x.csv"))).unwrap();
        let r = richiesta(Modo::Salva, &dir, "x.csv", &[FILTRO_CSV]);
        assert_eq!(risolvi(&*m, r), Some(dir.join("x.csv")));
        let viste = m.viste.lock().unwrap();
        assert_eq!(viste.len(), 2);
        assert_eq!(viste[1].nome, "x.csv", "la seconda parte dal nome completo");
        // Senza conflitto basta una sola finestra, con l'estensione aggiunta.
        let (m, tx) = finto(false);
        tx.send(Some(dir.join("nuovo"))).unwrap();
        let r = richiesta(Modo::Salva, &dir, "", &[FILTRO_CSV]);
        assert_eq!(risolvi(&*m, r), Some(dir.join("nuovo.csv")));
        // Aprendo il percorso non si tocca.
        let (m, tx) = finto(false);
        tx.send(Some(dir.join("senza_estensione"))).unwrap();
        let r = richiesta(Modo::Apri, &dir, "", &[FILTRO_CSV]);
        assert_eq!(risolvi(&*m, r), Some(dir.join("senza_estensione")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn richiesta_dalla_scelta() {
        let dir = cartella_prova("richiesta");
        let s = SceltaFile::new("Esporta", "Esporta", &dir, "s.csv")
            .salva()
            .filtri(&[FILTRO_CSV]);
        let r = s.richiesta();
        assert_eq!(r.modo, Modo::Salva);
        assert_eq!(r.nome, "s.csv");
        assert_eq!(r.cartella, dir);
        assert_eq!(s.finali, vec![".csv"]);
        // Con uno scopo mai usato resta la cartella data.
        let s = s.scopo(Scopo::EsportaCruscotto);
        assert_eq!(s.cartella_iniziale, dir);
        // Dopo una conferma, lo stesso scopo riparte da lì.
        let altra = cartella_prova("richiesta2");
        memoria()
            .lock()
            .unwrap()
            .ricorda(Scopo::EsportaCruscotto, &altra.join("f.json"));
        let s = SceltaFile::new("E", "Esporta", &dir, "s.json").scopo(Scopo::EsportaCruscotto);
        assert_eq!(s.cartella_iniziale, altra);
        assert_eq!(PathBuf::from(&s.percorso), altra.join("s.json"));
        memoria()
            .lock()
            .unwrap()
            .cartelle
            .remove(&Scopo::EsportaCruscotto);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&altra);
    }
}
