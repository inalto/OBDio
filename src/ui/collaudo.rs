//! Collaudo automatico dell'interfaccia in demo (solo sviluppo, feature
//! `collaudo`): con la variabile d'ambiente `OBDIO_COLLAUDO=<cartella>` l'app
//! percorre da sola il flusso demo e salva una schermata per ogni passo.

use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;

use super::cruscotto::modello::{Cella, Pagina, SPIA_MIL, Soglia, Strumento, TipoStrumento};
use super::{App, Scheda, VistaSensori};
use crate::elm::ConfermaCancellazione;
use crate::obd::pids;
use crate::recording;
use crate::scambio::FormatoEsportazione;
use crate::transport::demo::DemoTransport;
use crate::worker::{Comando, ComandoRiproduzione, Sorgente};

/// Fase iniziale del simulatore per il cruscotto: la velocità del ciclo demo
/// è zero nei primi 45 s e sale a 69 km/h fra 45 e 48 s; partendo da 40 s il
/// grafico della velocità mostra la salita.
const FASE_CRUSCOTTO_S: f64 = 40.0;

/// Pagina di prova con uno strumento per tipo, più uno su un parametro non
/// supportato dal simulatore e uno sulla sonda a valle (che ogni minuto
/// smette di rispondere per 2,5 s).
/// Parametro, tipo e (colonna, riga, larghezza, altezza).
type Voce = (&'static str, TipoStrumento, (u32, u32, u32, u32));

fn pagina_prova(primo_id: u64) -> Pagina {
    use TipoStrumento::*;
    let voci: [Voce; 11] = [
        ("giri", Quadrante, (0, 0, 3, 3)),
        ("temp_refrigerante", Arco, (3, 0, 3, 2)),
        ("velocita", Numerico, (6, 0, 3, 2)),
        ("contachilometri", Contachilometri, (9, 0, 3, 1)),
        (SPIA_MIL, Spia, (9, 1, 3, 1)),
        ("stft_b1", Barra, (3, 2, 3, 1)),
        ("o2s2_tensione", Grafico, (6, 2, 5, 2)),
        ("carico", Barra, (11, 2, 1, 3)),
        ("temp_olio", Numerico, (0, 3, 3, 2)),
        ("o2s2_tensione", Quadrante, (3, 3, 3, 3)),
        ("giri", Spia, (6, 4, 5, 1)),
    ];
    let strumenti = voci
        .iter()
        .enumerate()
        .map(|(i, (k, t, c))| {
            let mut s = Strumento::predefinito(k, *t);
            s.id = primo_id + i as u64;
            s.imposta_cella(Cella::new(c.0, c.1, c.2, c.3));
            if *t == Spia && *k == "giri" {
                s.titolo = Some("Giri oltre 2.000".into());
                s.soglia_attenzione = Some(Soglia::sopra(2000.0));
                s.soglia_critica = None;
            }
            if *t == Grafico {
                s.secondi = 30.0;
            }
            s
        })
        .collect();
    Pagina {
        nome: "Prova strumenti".into(),
        strumenti,
    }
}

fn sonda_a_valle_muta(app: &App) -> bool {
    app.live
        .ultimo("o2s2_tensione")
        .is_some_and(|p| p.v.is_none())
}

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
        // Cruscotto: nuovo collegamento demo a metà ciclo (velocità > 0).
        25 if !app.collegato() && c.secondi() > 0.5 => {
            app.invia(Comando::Collega(Sorgente::Trasporto(Box::new(
                DemoTransport::new().con_fase(FASE_CRUSCOTTO_S),
            ))));
            c.avanti();
        }
        26 if app.pronto() => {
            app.invia(Comando::LeggiDiagnosi);
            c.avanti();
        }
        // Aprendo il cruscotto la lettura parte da sola.
        27 if app.diagnosi.is_some() && app.pronto() => {
            app.cruscotto.pagina = 0;
            app.scheda = Scheda::Cruscotto;
            c.avanti();
        }
        28 if app.polling
            && app.live.t_max > 22.0
            && app
                .live
                .ultimo("velocita")
                .and_then(|p| p.v)
                .is_some_and(|v| v > 30.0) =>
        {
            scatta(&mut c, "9_cruscotto_guida")
        }
        29 => {
            app.cruscotto.pagina = 2;
            c.avanti();
        }
        30 if c.secondi() > 7.0 => scatta(&mut c, "10_cruscotto_p0420"),
        31 => {
            app.cruscotto.pagina = 0;
            app.cruscotto.modifica = true;
            app.cruscotto.selezionato = app.cruscotto.correnti.pagine[0]
                .strumenti
                .iter()
                .find(|s| s.chiave == "velocita" && s.tipo == TipoStrumento::Quadrante)
                .map(|s| s.id);
            // Una modifica non salvata, per mostrarne l'indicazione.
            if let Some(s) = app.cruscotto.correnti.pagine[0]
                .strumenti
                .iter_mut()
                .find(|s| s.chiave == "velocita" && s.tipo == TipoStrumento::Quadrante)
            {
                s.mostra_picco = true;
                s.titolo = Some("Velocità del veicolo".into());
            }
            c.avanti();
        }
        32 if c.secondi() > 1.0 => scatta(&mut c, "11_cruscotto_modifica"),
        33 => {
            app.cruscotto.annulla();
            let id = app.cruscotto.correnti.nuovo_id();
            app.cruscotto.correnti.pagine.push(pagina_prova(id));
            app.cruscotto.pagina = app.cruscotto.correnti.pagine.len() - 1;
            c.avanti();
        }
        // Attende che la sonda a valle smetta di rispondere (stato mancante).
        34 if c.secondi() > 2.0 && sonda_a_valle_muta(app) => c.avanti(),
        35 if c.secondi() > 0.9 => scatta(&mut c, "12_cruscotto_strumenti"),
        // Layout stretto: finestra da telefono.
        36 => {
            app.cruscotto.pagina = 0;
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
                360.0, 560.0,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(420.0, 820.0)));
            c.avanti();
        }
        37 if c.secondi() > 2.0 => scatta(&mut c, "13_cruscotto_stretto"),
        38 => {
            app.cruscotto.modifica = true;
            app.cruscotto.selezionato = app.cruscotto.correnti.pagine[0]
                .strumenti
                .first()
                .map(|s| s.id);
            c.avanti();
        }
        39 if c.secondi() > 1.0 => scatta(&mut c, "14_cruscotto_stretto_modifica"),
        40 => {
            app.cruscotto.annulla();
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1280.0, 820.0)));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
                980.0, 620.0,
            )));
            // Lasciando il cruscotto la lettura avviata da lui si ferma.
            app.scheda = Scheda::Sensori;
            c.avanti();
        }
        41 if c.secondi() > 1.5 => {
            if app.polling {
                eprintln!("collaudo: la lettura avviata dal cruscotto non si è fermata");
            }
            scatta(&mut c, "15_dopo_il_cruscotto")
        }
        42 => {
            app.invia(Comando::Scollega);
            c.avanti();
        }
        // Impostazioni, con valori d'esempio (nulla viene salvato né applicato).
        43 if !app.collegato() && c.secondi() > 0.5 => {
            let f = &mut app.impostazioni.form;
            f.attivo = true;
            f.host = "192.168.1.10".into();
            f.utente = "obdio".into();
            f.password = "esempio".into();
            f.veicolo = "Fiat Panda".into();
            f.discovery_home_assistant = true;
            app.scheda = Scheda::Impostazioni;
            c.avanti();
        }
        44 if c.secondi() > 0.6 => scatta(&mut c, "9_impostazioni"),
        // Esportazione della sessione demo registrata in CSV stile Torque.
        45 => {
            app.scheda = Scheda::Sessioni;
            let torque = c.cartella.join("sessione_demo_torque.csv");
            app.esporta_sessione(FormatoEsportazione::CsvTorque, &torque);
            if app
                .sessioni
                .esito
                .as_ref()
                .is_none_or(|e| e.cartella.is_none())
            {
                eprintln!("collaudo: esportazione Torque non riuscita");
            }
            c.avanti();
        }
        // Reimportazione dello stesso file: anteprima con il riepilogo.
        46 if c.secondi() > 0.5 => {
            app.importa_da(c.cartella.join("sessione_demo_torque.csv"));
            c.avanti();
        }
        47 if app.sessioni.anteprima.is_some() && c.secondi() > 0.8 => {
            scatta(&mut c, "16_importazione_anteprima")
        }
        48 => {
            app.conferma_importazione();
            c.avanti();
        }
        49 if c.secondi() > 1.0 => {
            if app.aperta.as_ref().is_none_or(|s| s.riepilogo.is_none()) {
                eprintln!("collaudo: la sessione importata non è stata aperta");
            }
            scatta(&mut c, "17_sessione_importata")
        }
        // Riproduzione della sessione demo con il cruscotto «Guida».
        50 => {
            match app.ultima.as_ref().map(|u| recording::carica(&u.percorso)) {
                Some(Ok(dati)) => {
                    app.cruscotto.pagina = 0;
                    app.avvia_riproduzione(dati);
                }
                _ => eprintln!("collaudo: sessione registrata non riapribile"),
            }
            c.avanti();
        }
        // Verso la fine del minimo: poco dopo la velocità sale.
        51 if app.riproduzione.is_some() && c.secondi() > 0.5 => {
            app.invia(Comando::Riproduzione(ComandoRiproduzione::VaiA(40.0)));
            app.invia(Comando::Riproduzione(ComandoRiproduzione::Velocita(2.0)));
            c.avanti();
        }
        52 if c.secondi() > 2.6 => scatta(&mut c, "18_riproduzione_guida"),
        // Salto al marcatore «2.500 giri» e pausa.
        53 => {
            let t = app.riproduzione.as_ref().and_then(|r| {
                r.marcatori
                    .iter()
                    .find(|m| m.1 == "2.500 giri")
                    .map(|m| m.0)
            });
            match t {
                Some(t) => app.invia(Comando::Riproduzione(ComandoRiproduzione::VaiA(t))),
                None => eprintln!("collaudo: marcatore «2.500 giri» assente"),
            }
            c.avanti();
        }
        54 if c.secondi() > 1.5 => {
            app.invia(Comando::Riproduzione(ComandoRiproduzione::Pausa));
            app.scheda = Scheda::Sensori;
            app.vista = VistaSensori::PerSensore;
            app.scorri_sensori = Some(0.0);
            c.avanti();
        }
        55 if c.secondi() > 1.0 => scatta(&mut c, "19_riproduzione_sensori_in_pausa"),
        56 => {
            app.vista = VistaSensori::Confronto;
            app.invia(Comando::Scollega);
            c.avanti();
        }
        57 if !app.collegato() && c.secondi() > 0.5 => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            c.avanti();
        }
        _ => {}
    }
    app.collaudo = Some(c);
}
