//! Collaudo automatico dell'interfaccia in demo (solo sviluppo, feature
//! `collaudo`): con la variabile d'ambiente `OBDIO_COLLAUDO=<cartella>` l'app
//! percorre da sola il flusso demo e salva le schermate di ogni destinazione,
//! larga (1280 × 820, `largo_*.png`) e da telefono (390 × 844, `stretto_*.png`).

use std::collections::VecDeque;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;

use super::cruscotto::modello::{Cella, Pagina, SPIA_MIL, Soglia, Strumento, TipoStrumento};
use super::impostazioni_aggiornamenti::InCorso;
use super::sessioni::AzioneSessioni;
use super::{App, PannelloSensori, Scheda, VistaSensori};
use crate::aggiornamenti::{Rilascio, Valutazione, Versione, rete};
use crate::elm::ConfermaCancellazione;
use crate::mcp::{Livello, McpConfig, StatoMcp, StatoMcpInfo};
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

/// Scheda del server MCP come appare da acceso, con valori d'esempio: nulla
/// viene salvato e nessuna porta viene aperta.
fn esempio_mcp(app: &mut App) {
    let m = &mut app.mcp;
    m.config = McpConfig {
        attivo: true,
        porta: 8765,
        token: "5e1f0c9a7b3d42e86a90d4c1f7b25e03c8a6149d2f7e0b53a1c4d98e6f207b3d".into(),
        livello: Livello::SolaLettura,
    };
    m.porta = 8765;
    m.mostra_token = false;
    m.messaggio = None;
    m.info = StatoMcpInfo {
        stato: StatoMcp::InAscolto(8765),
        richieste: 128,
        rifiutate: 1,
        ultimo_client: Some("claude-code 2.1.0".into()),
        ultimo_errore: None,
    };
}

/// Scheda Aggiornamenti come appare quando GitHub offre una versione nuova:
/// rilascio d'esempio (la minore successiva a quella in uso), nessuna
/// richiesta in rete, nulla salvato su disco.
fn esempio_aggiornamento(app: &mut App) {
    let v = Versione::corrente();
    let nuova = format!("{}.{}.0", v.maggiore, v.minore + 1);
    let note = serde_json::to_string(
        "Novità\r\n\r\n\
         - Controllo degli aggiornamenti da GitHub, con interruttore nelle Impostazioni.\r\n\
         - Installazione con un clic su Windows, con verifica della somma SHA-256.\r\n\
         - Descrizioni italiane per altri codici guasto generici.\r\n\r\n\
         Correzioni\r\n\r\n\
         - Il cruscotto ricorda la pagina aperta.\r\n\
         - La riproduzione riparte dal marcatore scelto anche in pausa.\r\n\r\n\
         Dettagli: https://github.com/inalto/OBDio/blob/main/docs/NOVITA.md",
    )
    .unwrap_or_default();
    let json = format!(
        r#"{{
  "tag_name": "v{nuova}",
  "name": "OBDio {nuova}",
  "html_url": "https://github.com/inalto/OBDio/releases/tag/v{nuova}",
  "draft": false,
  "prerelease": false,
  "published_at": "2026-12-01T10:00:00Z",
  "body": {note},
  "assets": [
    {{"name": "OBDio-{nuova}-windows-x64.zip", "size": 5242880, "state": "uploaded",
      "browser_download_url": "https://github.com/inalto/OBDio/releases/download/v{nuova}/OBDio-{nuova}-windows-x64.zip"}},
    {{"name": "OBDio-{nuova}-windows-x64.exe", "size": 12582912, "state": "uploaded",
      "browser_download_url": "https://github.com/inalto/OBDio/releases/download/v{nuova}/OBDio-{nuova}-windows-x64.exe"}},
    {{"name": "SHA256SUMS.txt", "size": 190, "state": "uploaded",
      "browser_download_url": "https://github.com/inalto/OBDio/releases/download/v{nuova}/SHA256SUMS.txt"}}
  ]
}}"#
    );
    let r = match Rilascio::da_json(&json) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("collaudo: rilascio d'esempio non valido: {e}");
            return;
        }
    };
    let a = &mut app.aggiornamenti;
    a.config.ultimo_controllo = Some(rete::adesso() - 180);
    a.config.ultimo_esito = Some(format!("disponibile la versione {nuova}"));
    a.rilascio = Some(Box::new(r));
    a.valutazione = Some(Valutazione::Disponibile);
    a.piu_tardi = false;
    a.installato = None;
    a.messaggio = None;
    a.come_installata = true;
    a.fissa_alla_scheda = true;
}

fn sonda_a_valle_muta(app: &App) -> bool {
    app.live
        .ultimo("o2s2_tensione")
        .is_some_and(|p| p.v.is_none())
}

/// Misure della finestra per le schermate: larga e da telefono.
const LARGO: [f32; 2] = [1280.0, 820.0];
const STRETTO: [f32; 2] = [390.0, 844.0];

/// Azioni in coda fra un passo e l'altro.
enum Azione {
    /// Schermata con questo nome.
    Scatta(String),
    /// Cambia la misura interna della finestra.
    Misura([f32; 2]),
    /// Attende che il layout si assesti.
    Attendi(f32),
}

pub struct Collaudo {
    cartella: PathBuf,
    fase: u32,
    da: Instant,
    scatto: Option<String>,
    coda: VecDeque<Azione>,
    attesa: Instant,
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
            coda: VecDeque::new(),
            attesa: Instant::now(),
        })
    }

    fn avanti(&mut self) {
        self.fase += 1;
        self.da = Instant::now();
    }

    fn secondi(&self) -> f32 {
        self.da.elapsed().as_secs_f32()
    }

    /// Una schermata nella misura corrente (larga).
    fn scatta(&mut self, nome: &str) {
        self.coda.push_back(Azione::Scatta(nome.to_string()));
    }

    /// Solo la misura da telefono, poi di nuovo larga.
    fn stretto(&mut self, nome: &str) {
        self.coda.extend([
            Azione::Misura(STRETTO),
            Azione::Attendi(1.2),
            Azione::Scatta(format!("stretto_{nome}")),
            Azione::Misura(LARGO),
            Azione::Attendi(1.0),
        ]);
    }

    /// La stessa vista larga e da telefono.
    fn doppio(&mut self, nome: &str) {
        self.scatta(&format!("largo_{nome}"));
        self.stretto(nome);
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

/// Esegue la coda delle azioni; `true` finché c'è qualcosa in corso. Svuotata
/// la coda, il collaudo passa al passo successivo.
fn esegui_coda(c: &mut Collaudo, ctx: &egui::Context) -> bool {
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
            c.attesa = Instant::now();
            if c.coda.is_empty() {
                c.avanti();
            }
        }
        return true;
    }
    let Some(azione) = c.coda.front() else {
        return false;
    };
    match azione {
        Azione::Scatta(nome) => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            c.scatto = Some(nome.clone());
            c.coda.pop_front();
        }
        Azione::Misura(m) => {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(m[0], m[1])));
            c.coda.pop_front();
            c.attesa = Instant::now();
        }
        Azione::Attendi(s) => {
            if c.attesa.elapsed().as_secs_f32() > *s {
                c.coda.pop_front();
                c.attesa = Instant::now();
                if c.coda.is_empty() {
                    c.avanti();
                }
            }
        }
    }
    true
}

pub fn passo(app: &mut App, ctx: &egui::Context) {
    let Some(mut c) = app.collaudo.take() else {
        return;
    };
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    if esegui_coda(&mut c, ctx) {
        app.collaudo = Some(c);
        return;
    }

    match c.fase {
        0 if c.secondi() > 1.0 => c.doppio("connessione"),
        1 => {
            app.invia(Comando::Collega(Sorgente::Demo));
            c.avanti();
        }
        2 if app.pronto() => {
            app.invia(Comando::LeggiDiagnosi);
            app.scheda = Scheda::Diagnosi;
            c.avanti();
        }
        3 if app.diagnosi.is_some() && c.secondi() > 0.8 => c.doppio("diagnosi"),
        4 => {
            app.scorri_diagnosi = Some(250.0);
            c.avanti();
        }
        5 if c.secondi() > 0.6 => c.scatta("largo_fermo_immagine"),
        6 => {
            app.scorri_diagnosi = Some(0.0);
            app.scheda = Scheda::Sensori;
            app.pannello_sensori = PannelloSensori::Elenco;
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
        9 if app.live.t_max > 56.0 => c.doppio("sensori"),
        10 => {
            app.pannello_sensori = PannelloSensori::Grafici;
            c.stretto("sensori_grafici");
        }
        // Un grafico per sensore: diciassette sensori selezionati, in cima e poi in fondo.
        11 => {
            app.vista = VistaSensori::PerSensore;
            c.avanti();
        }
        12 if c.secondi() > 1.0 => c.doppio("sensori_per_sensore"),
        13 => {
            app.scorri_sensori = Some(10_000.0);
            c.avanti();
        }
        14 if c.secondi() > 0.8 => c.scatta("largo_sensori_per_sensore_in_fondo"),
        15 => {
            app.vista = VistaSensori::Confronto;
            app.pannello_sensori = PannelloSensori::Elenco;
            app.invia(Comando::FermaRegistrazione);
            c.avanti();
        }
        16 if app.ultima.is_some() && c.secondi() > 0.5 => {
            c.scatta("largo_sensori_sessione_salvata")
        }
        17 => {
            if let Some(u) = &app.ultima {
                app.apri_sessione(u.percorso.clone());
            }
            app.sessioni.dettaglio = true;
            app.scheda = Scheda::Sessioni;
            c.avanti();
        }
        18 if app.aperta.is_some() && c.secondi() > 0.8 => c.doppio("sessioni"),
        19 => {
            app.sessioni.dettaglio = false;
            c.stretto("sessioni_elenco");
        }
        20 => {
            app.scheda = Scheda::Connessione;
            c.avanti();
        }
        21 if c.secondi() > 0.5 => c.doppio("connessione_collegato"),
        // Sullo stretto il pulsante di cancellazione sta in fondo alla diagnosi:
        // prima si restringe la finestra, poi si scorre.
        22 => {
            app.scheda = Scheda::Diagnosi;
            c.coda
                .extend([Azione::Misura(STRETTO), Azione::Attendi(1.2)]);
        }
        23 => {
            app.scorri_diagnosi = Some(10_000.0);
            c.coda.extend([
                Azione::Attendi(0.8),
                Azione::Scatta("stretto_diagnosi_in_fondo".into()),
                Azione::Misura(LARGO),
                Azione::Attendi(1.0),
            ]);
        }
        // Cancellazione dei codici sul simulatore: conferma, poi esito.
        24 => {
            app.scorri_diagnosi = Some(0.0);
            app.conferma_cancellazione = Some(true);
            c.avanti();
        }
        25 if c.secondi() > 0.6 => c.doppio("conferma_cancellazione"),
        26 => {
            app.conferma_cancellazione = None;
            app.invia(Comando::CancellaDtc(
                ConfermaCancellazione::utente_ha_confermato(),
            ));
            c.avanti();
        }
        27 if app.esito_cancellazione.is_some()
            && app.pronto()
            && app.diagnosi.as_ref().is_some_and(|d| !d.fermo.presente)
            && c.secondi() > 0.8 =>
        {
            c.doppio("esito_cancellazione")
        }
        28 => {
            app.invia(Comando::Scollega);
            c.avanti();
        }
        // Cruscotto: nuovo collegamento demo a metà ciclo (velocità > 0).
        29 if !app.collegato() && c.secondi() > 0.5 => {
            app.invia(Comando::Collega(Sorgente::Trasporto(Box::new(
                DemoTransport::new().con_fase(FASE_CRUSCOTTO_S),
            ))));
            c.avanti();
        }
        30 if app.pronto() => {
            app.invia(Comando::LeggiDiagnosi);
            c.avanti();
        }
        // Aprendo il cruscotto la lettura parte da sola.
        31 if app.diagnosi.is_some() && app.pronto() => {
            app.cruscotto.pagina = 0;
            app.scheda = Scheda::Cruscotto;
            c.avanti();
        }
        32 if app.polling
            && app.live.t_max > 22.0
            && app
                .live
                .ultimo("velocita")
                .and_then(|p| p.v)
                .is_some_and(|v| v > 30.0) =>
        {
            c.doppio("cruscotto_guida")
        }
        33 => {
            app.cruscotto.pagina = 2;
            c.avanti();
        }
        34 if c.secondi() > 7.0 => c.doppio("cruscotto_p0420"),
        35 => {
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
        36 if c.secondi() > 1.0 => c.doppio("cruscotto_modifica"),
        // Chiusura della finestra con il cruscotto non salvato: l'app la
        // intercetta e chiede che cosa fare.
        37 => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            c.avanti();
        }
        38 if app.chiedi_chiusura && c.secondi() > 0.6 => c.doppio("chiusura_non_salvato"),
        39 => {
            app.chiedi_chiusura = false;
            app.cruscotto.annulla();
            let id = app.cruscotto.correnti.nuovo_id();
            app.cruscotto.correnti.pagine.push(pagina_prova(id));
            app.cruscotto.pagina = app.cruscotto.correnti.pagine.len() - 1;
            c.avanti();
        }
        // Attende che la sonda a valle smetta di rispondere (stato mancante).
        40 if c.secondi() > 2.0 && sonda_a_valle_muta(app) => c.avanti(),
        41 if c.secondi() > 0.9 => c.doppio("cruscotto_strumenti"),
        42 => {
            app.cruscotto.annulla();
            // Lasciando il cruscotto la lettura avviata da lui si ferma.
            app.scheda = Scheda::Sensori;
            c.avanti();
        }
        43 if c.secondi() > 1.5 => {
            if app.polling {
                eprintln!("collaudo: la lettura avviata dal cruscotto non si è fermata");
            }
            app.invia(Comando::Scollega);
            c.avanti();
        }
        // Impostazioni, con valori d'esempio (nulla viene salvato né applicato).
        44 if !app.collegato() && c.secondi() > 0.5 => {
            let f = &mut app.impostazioni.form;
            f.attivo = true;
            f.host = "192.168.1.10".into();
            f.utente = "obdio".into();
            f.password = "esempio".into();
            f.veicolo = "Fiat Panda".into();
            f.discovery_home_assistant = true;
            // La scheda del server MCP sta in cima: si scorre fino a MQTT.
            app.mcp.oltre_la_scheda = true;
            app.scheda = Scheda::Impostazioni;
            c.avanti();
        }
        45 if c.secondi() > 0.6 => c.doppio("impostazioni"),
        // Finestra di scelta del file (importazione).
        46 => {
            app.scheda = Scheda::Sessioni;
            app.apri_scelta_sessioni(AzioneSessioni::Importa);
            c.avanti();
        }
        47 if c.secondi() > 0.6 => c.doppio("scelta_file"),
        // Esportazione della sessione demo registrata in CSV stile Torque.
        48 => {
            app.sessioni.dialogo = None;
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
        49 if c.secondi() > 0.5 => {
            app.importa_da(c.cartella.join("sessione_demo_torque.csv"));
            c.avanti();
        }
        50 if app.sessioni.anteprima.is_some() && c.secondi() > 0.8 => {
            c.doppio("importazione_anteprima")
        }
        51 => {
            app.conferma_importazione();
            app.sessioni.dettaglio = true;
            c.avanti();
        }
        52 if c.secondi() > 1.0 => {
            if app.aperta.as_ref().is_none_or(|s| s.riepilogo.is_none()) {
                eprintln!("collaudo: la sessione importata non è stata aperta");
            }
            c.doppio("sessione_importata")
        }
        // Riproduzione della sessione demo con il cruscotto «Guida».
        53 => {
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
        54 if app.riproduzione.is_some() && c.secondi() > 0.5 => {
            app.invia(Comando::Riproduzione(ComandoRiproduzione::VaiA(40.0)));
            app.invia(Comando::Riproduzione(ComandoRiproduzione::Velocita(2.0)));
            c.avanti();
        }
        55 if c.secondi() > 2.6 => c.doppio("riproduzione_guida"),
        // Salto al marcatore «2.500 giri» e pausa.
        56 => {
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
        57 if c.secondi() > 1.5 => {
            app.invia(Comando::Riproduzione(ComandoRiproduzione::Pausa));
            app.scheda = Scheda::Sensori;
            app.vista = VistaSensori::PerSensore;
            app.pannello_sensori = PannelloSensori::Grafici;
            app.scorri_sensori = Some(0.0);
            c.avanti();
        }
        58 if c.secondi() > 1.0 => c.doppio("riproduzione_sensori_in_pausa"),
        59 => {
            app.vista = VistaSensori::Confronto;
            app.invia(Comando::Scollega);
            c.avanti();
        }
        // Scheda del server MCP accesa, con valori d'esempio e token mascherato.
        60 if !app.collegato() && c.secondi() > 0.5 => {
            esempio_mcp(app);
            app.mcp.oltre_la_scheda = false;
            app.scheda = Scheda::Impostazioni;
            app.mcp.scorri = Some(0.0);
            c.avanti();
        }
        61 if c.secondi() > 0.6 => c.doppio("mcp"),
        // Il resto della scheda: configurazione del client e spiegazione.
        62 => {
            app.mcp.scorri = Some((app.mcp.altezza - 640.0).max(0.0));
            c.coda.extend([
                Azione::Attendi(0.6),
                Azione::Scatta("largo_mcp_in_fondo".into()),
                Azione::Misura(STRETTO),
                Azione::Attendi(1.2),
            ]);
        }
        63 => {
            app.mcp.scorri = Some(app.mcp.altezza * 0.34);
            c.coda.extend([
                Azione::Attendi(0.8),
                Azione::Scatta("stretto_mcp_al_centro".into()),
            ]);
        }
        64 => {
            app.mcp.scorri = Some((app.mcp.altezza - 620.0).max(0.0));
            c.coda.extend([
                Azione::Attendi(0.8),
                Azione::Scatta("stretto_mcp_in_fondo".into()),
                Azione::Misura(LARGO),
                Azione::Attendi(1.0),
            ]);
        }
        // Scheda Aggiornamenti con una versione nuova d'esempio: nessuna
        // richiesta in rete, nulla salvato.
        65 => {
            esempio_aggiornamento(app);
            app.scheda = Scheda::Impostazioni;
            c.avanti();
        }
        66 if c.secondi() > 0.8 => c.scatta("largo_aggiornamenti"),
        // Sullo stretto la scheda è più alta dello schermo: l'inizio, poi il
        // riquadro della versione con i pulsanti.
        67 => c.coda.extend([
            Azione::Misura(STRETTO),
            Azione::Attendi(1.2),
            Azione::Scatta("stretto_aggiornamenti_scheda".into()),
        ]),
        68 => {
            app.aggiornamenti.fissa_al_riquadro = true;
            c.coda.extend([
                Azione::Attendi(0.8),
                Azione::Scatta("stretto_aggiornamenti".into()),
            ]);
        }
        69 => {
            app.aggiornamenti.in_corso = Some(InCorso::Scaricamento {
                scaricati: 5_400_000,
                totale: 12_582_912,
            });
            c.coda.extend([
                Azione::Attendi(0.6),
                Azione::Scatta("stretto_aggiornamenti_scaricamento".into()),
                Azione::Misura(LARGO),
                Azione::Attendi(1.0),
            ]);
        }
        70 => {
            app.aggiornamenti.fissa_al_riquadro = false;
            c.coda.extend([
                Azione::Attendi(0.6),
                Azione::Scatta("largo_aggiornamenti_scaricamento".into()),
            ]);
        }
        71 => {
            app.aggiornamenti.in_corso = None;
            app.aggiornamenti.fissa_alla_scheda = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            c.avanti();
        }
        _ => {}
    }
    app.collaudo = Some(c);
}
