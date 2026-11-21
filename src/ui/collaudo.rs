//! Collaudo automatico dell'interfaccia in demo (solo sviluppo, feature
//! `collaudo`): con la variabile d'ambiente `OBDIO_COLLAUDO=<cartella>` l'app
//! percorre da sola il flusso demo e salva una schermata per ogni passo.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;

use super::{App, Scheda, VistaSensori};
use crate::elm::ConfermaCancellazione;
use crate::obd::pids;
use crate::worker::{Comando, Sorgente};

pub struct Collaudo {
    cartella: PathBuf,
    fase: u32,
    da: Instant,
    scatto: Option<String>,
}

impl Collaudo {
    pub fn da_ambiente() -> Option<Self> {
        let cartella = PathBuf::from(std::env::var_os("OBDIO_COLLAUDO")?);
        std::fs::create_dir_all(&cartella).ok()?;
        Some(Collaudo {
            cartella,
            fase: 0,
            da: Instant::now(),
            scatto: None,
        })
    }

    fn avanti(&mut self) {
        self.fase += 1;
        self.da = Instant::now();
    }

    fn secondi(&self) -> f32 {
        self.da.elapsed().as_secs_f32()
    }
}

fn salva(percorso: &std::path::Path, img: &egui::ColorImage) -> Result<(), String> {
    let file = File::create(percorso).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(BufWriter::new(file), img.size[0] as u32, img.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(img.as_raw()).map_err(|e| e.to_string())
}

pub fn passo(app: &mut App, ctx: &egui::Context) {
    let Some(mut c) = app.collaudo.take() else {
        return;
    };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));

    // Schermata richiesta in un fotogramma precedente.
    if let Some(nome) = c.scatto.clone() {
        let immagine = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = immagine {
            let percorso = c.cartella.join(format!("{nome}.png"));
            if let Err(e) = salva(&percorso, &img) {
                eprintln!("collaudo: schermata non salvata: {e}");
            }
            c.scatto = None;
            c.avanti();
        }
        app.collaudo = Some(c);
        return;
    }
    let scatta = |c: &mut Collaudo, nome: &str| {
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        c.scatto = Some(nome.to_string());
    };

    match c.fase {
        0 if c.secondi() > 1.0 => scatta(&mut c, "0_connessione"),
        1 => {
            app.invia(Comando::Collega(Sorgente::Demo));
            c.avanti();
        }
        2 if app.pronto() => {
            app.invia(Comando::LeggiDiagnosi);
            app.scheda = Scheda::Diagnosi;
            c.avanti();
        }
        3 if app.diagnosi.is_some() && c.secondi() > 0.8 => scatta(&mut c, "1_diagnosi"),
        4 => {
            app.scorri_diagnosi = Some(520.0);
            c.avanti();
        }
        5 if c.secondi() > 0.6 => scatta(&mut c, "2_fermo_immagine"),
        6 => {
            app.scheda = Scheda::Sensori;
            // Oltre al preset P0420 (7 parametri): altri 9 sensori reali del
            // simulatore e la distanza parziale calcolata.
            for k in [
                "carico",
                "map",
                "temp_aspirazione",
                "anticipo",
                "farfalla",
                "pressione_barometrica",
                "tensione_modulo",
                "lambda_comandato",
                "tempo_motore",
                pids::CHIAVE_DISTANZA_PARZIALE,
            ] {
                app.selezione.insert(k.to_string(), true);
            }
            app.extra = Some("giri".into());
            app.avvia_polling();
            app.invia(Comando::AvviaRegistrazione);
            c.avanti();
        }
        7 if c.secondi() > 5.0 => {
            app.invia(Comando::Marcatore("Minimo".into()));
            c.avanti();
        }
        8 if c.secondi() > 24.0 => {
            app.invia(Comando::Marcatore("2.500 giri".into()));
            c.avanti();
        }
        // Oltre il secondo 50 la demo ha simulato un'interruzione della sonda a valle.
        9 if app.live.t_max > 56.0 => scatta(&mut c, "3_tempo_reale"),
        // Un grafico per sensore: dieci sensori selezionati, in cima e poi in fondo.
        10 => {
            app.vista = VistaSensori::PerSensore;
            c.avanti();
        }
        11 if c.secondi() > 1.0 => scatta(&mut c, "3b_un_grafico_per_sensore"),
        12 => {
            app.scorri_sensori = Some(10_000.0);
            c.avanti();
        }
        13 if c.secondi() > 0.8 => scatta(&mut c, "3c_un_grafico_per_sensore_in_fondo"),
        14 => {
            app.vista = VistaSensori::Confronto;
            app.invia(Comando::FermaRegistrazione);
            c.avanti();
        }
        15 if app.ultima.is_some() && c.secondi() > 0.5 => scatta(&mut c, "4_sessione_salvata"),
        16 => {
            if let Some(u) = &app.ultima {
                app.apri_sessione(u.percorso.clone());
            }
            app.scheda = Scheda::Sessioni;
            c.avanti();
        }
        17 if app.aperta.is_some() && c.secondi() > 0.8 => scatta(&mut c, "5_sessione_riaperta"),
        18 => {
            app.scheda = Scheda::Connessione;
            c.avanti();
        }
        19 if c.secondi() > 0.5 => scatta(&mut c, "6_collegato"),
        // Cancellazione dei codici sul simulatore: conferma, poi esito.
        20 => {
            app.scheda = Scheda::Diagnosi;
            app.conferma_cancellazione = Some(true);
            c.avanti();
        }
        21 if c.secondi() > 0.6 => scatta(&mut c, "7_conferma_cancellazione"),
        22 => {
            app.conferma_cancellazione = None;
            app.invia(Comando::CancellaDtc(
                ConfermaCancellazione::utente_ha_confermato(),
            ));
            c.avanti();
        }
        23 if app.esito_cancellazione.is_some()
            && app.pronto()
            && app.diagnosi.as_ref().is_some_and(|d| !d.fermo.presente)
            && c.secondi() > 0.8 =>
        {
            scatta(&mut c, "8_esito_cancellazione")
        }
        24 => {
            app.invia(Comando::Scollega);
            c.avanti();
        }
        25 if !app.collegato() && c.secondi() > 0.5 => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            c.avanti();
        }
        _ => {}
    }
    app.collaudo = Some(c);
}
