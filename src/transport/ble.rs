//! Bluetooth Low Energy tramite btleplug (API WinRT su Windows).
//!
//! Il collegamento procede per passi, ognuno con il suo tempo massimo e il
//! suo testo d'errore: connessione radio, scoperta dei servizi GATT, flusso
//! delle notifiche, attivazione delle notifiche. Ogni passo finisce nel
//! [`Diario`] (registro dei collegamenti su disco ed eco nell'app), così si
//! sa sempre quale passo è fallito o in quale passo il processo si è fermato.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use btleplug::api::{
    Central, CentralEvent, CentralState, CharPropFlags, Characteristic, Manager as _,
    Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::profili::{CaratteristicaVista, PROFILI, ProfiloBle, riconosci};
use super::{Scollegato, Transport};
use crate::registri::Diario;

/// Tempo massimo della connessione radio (per tentativo).
pub const T_COLLEGAMENTO: Duration = Duration::from_secs(15);
/// Tempo massimo della scoperta dei servizi e delle caratteristiche.
pub const T_SCOPERTA: Duration = Duration::from_secs(20);
/// Tempo massimo per aprire il flusso delle notifiche.
pub const T_FLUSSO: Duration = Duration::from_secs(5);
/// Tempo massimo per attivare le notifiche (scrittura del descrittore CCCD).
pub const T_ISCRIZIONE: Duration = Duration::from_secs(10);
/// Tempo massimo dello scollegamento.
pub const T_SCOLLEGAMENTO: Duration = Duration::from_secs(3);
/// Pausa fra l'arresto della scansione e la connessione.
const PAUSA_DOPO_SCANSIONE: Duration = Duration::from_millis(300);
/// Pausa prima dell'unico nuovo tentativo di connessione.
const PAUSA_PRIMA_DEL_RIPROVO: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug)]
pub enum BleErrore {
    /// Il PC non ha un adattatore Bluetooth utilizzabile.
    NessunAdattatore,
    /// Il Bluetooth è disattivato in Windows.
    BluetoothSpento,
    /// Il dispositivo scelto non risponde o rifiuta la connessione. Il testo
    /// inizia con il passo fallito.
    NonRaggiungibile(String),
    /// Il dispositivo rifiuta l'accesso GATT (accesso negato o errore di
    /// protocollo): di solito vuole l'associazione in Windows. Il testo
    /// inizia con il passo fallito.
    AccessoNegato(String),
    /// Connesso, ma i servizi GATT non corrispondono a un profilo documentato.
    ProfiloSconosciuto(GattInfo),
    Altro(String),
}

impl BleErrore {
    /// Testo breve per il registro.
    pub fn testo(&self) -> String {
        match self {
            BleErrore::NessunAdattatore => "nessun adattatore Bluetooth nel PC".into(),
            BleErrore::BluetoothSpento => "Bluetooth disattivato".into(),
            BleErrore::NonRaggiungibile(d) => format!("non raggiungibile: {d}"),
            BleErrore::AccessoNegato(d) => format!("accesso negato: {d}"),
            BleErrore::ProfiloSconosciuto(_) => "profilo GATT non riconosciuto".into(),
            BleErrore::Altro(d) => d.clone(),
        }
    }
}

/// Passo del collegamento.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaseBle {
    Collegamento,
    ScopertaServizi,
    FlussoNotifiche,
    AttivazioneNotifiche,
}

impl FaseBle {
    pub fn nome(self) -> &'static str {
        match self {
            FaseBle::Collegamento => "connessione radio",
            FaseBle::ScopertaServizi => "scoperta dei servizi GATT",
            FaseBle::FlussoNotifiche => "apertura del flusso delle notifiche",
            FaseBle::AttivazioneNotifiche => "attivazione delle notifiche",
        }
    }

    pub fn limite(self) -> Duration {
        match self {
            FaseBle::Collegamento => T_COLLEGAMENTO,
            FaseBle::ScopertaServizi => T_SCOPERTA,
            FaseBle::FlussoNotifiche => T_FLUSSO,
            FaseBle::AttivazioneNotifiche => T_ISCRIZIONE,
        }
    }
}

/// Causa probabile di un errore, dedotta dall'errore di btleplug o dal suo
/// testo (stati GATT e codici HRESULT di WinRT).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CausaBle {
    /// `GattCommunicationStatus::AccessDenied`, E_ACCESSDENIED o un errore
    /// ATT di autenticazione/cifratura: il dispositivo vuole l'associazione.
    AccessoNegato,
    /// `GattCommunicationStatus::ProtocolError` o un altro errore ATT.
    ErroreProtocollo,
    /// `GattCommunicationStatus::Unreachable`, dispositivo non trovato o non
    /// pronto.
    Irraggiungibile,
    /// Nessuna risposta entro il tempo massimo.
    TempoScaduto,
    Altro,
}

impl CausaBle {
    /// Vale la pena un secondo tentativo (una volta sola).
    pub fn transitoria(self) -> bool {
        matches!(self, CausaBle::Irraggiungibile | CausaBle::TempoScaduto)
    }

    pub fn nome(self) -> &'static str {
        match self {
            CausaBle::AccessoNegato => "accesso negato",
            CausaBle::ErroreProtocollo => "errore di protocollo GATT",
            CausaBle::Irraggiungibile => "dispositivo non raggiungibile",
            CausaBle::TempoScaduto => "tempo scaduto",
            CausaBle::Altro => "errore WinRT",
        }
    }
}

/// Classifica un errore di btleplug.
pub fn classifica(e: &btleplug::Error) -> CausaBle {
    match e {
        btleplug::Error::PermissionDenied => CausaBle::AccessoNegato,
        btleplug::Error::NotConnected | btleplug::Error::DeviceNotFound => {
            CausaBle::Irraggiungibile
        }
        btleplug::Error::TimedOut(_) => CausaBle::TempoScaduto,
        altro => classifica_testo(&format!("{altro} {altro:?}")),
    }
}

/// Classifica il testo di un errore (WinRT lo riporta come
/// `GattCommunicationStatus(n)` o `HRESULT(0x…)`).
pub fn classifica_testo(testo: &str) -> CausaBle {
    let t = testo.to_lowercase();
    let ha = |chiavi: &[&str]| chiavi.iter().any(|k| t.contains(k));
    // Errori ATT (0x8065xxxx): 05 autenticazione, 08 autorizzazione,
    // 0C chiave di cifratura troppo corta, 0F cifratura insufficiente.
    if ha(&[
        "gattcommunicationstatus(3)",
        "accessdenied",
        "access denied",
        "access is denied",
        "permission denied",
        "0x80070005",
        "0x80650005",
        "0x80650008",
        "0x8065000c",
        "0x8065000f",
    ]) {
        CausaBle::AccessoNegato
    } else if ha(&["gattcommunicationstatus(2)", "protocolerror", "0x8065"]) {
        CausaBle::ErroreProtocollo
    } else if ha(&[
        "gattcommunicationstatus(1)",
        "unreachable",
        "not connected",
        "device not found",
        "0x800710df",
        "0x8007048f",
        "0x80070490",
    ]) {
        CausaBle::Irraggiungibile
    } else if ha(&["timed out", "timeout", "0x800705b4"]) {
        CausaBle::TempoScaduto
    } else {
        CausaBle::Altro
    }
}

/// Errore di un passo, con la causa nel testo.
fn errore_di_fase(fase: FaseBle, causa: CausaBle, dettaglio: &str) -> BleErrore {
    let testo = format!("{}: {dettaglio} ({})", fase.nome(), causa.nome());
    match causa {
        CausaBle::AccessoNegato | CausaBle::ErroreProtocollo => BleErrore::AccessoNegato(testo),
        _ => BleErrore::NonRaggiungibile(testo),
    }
}

/// Esegue `f` nel runtime dal thread chiamante. Dall'interno del runtime
/// `block_on` andrebbe in panic: lì restituisce un errore.
fn blocca<F: Future>(rt: &Runtime, f: F) -> Result<F::Output, BleErrore> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(BleErrore::Altro(
            "chiamata Bluetooth bloccante dall'interno del runtime asincrono".into(),
        ));
    }
    Ok(rt.block_on(f))
}

/// Dispositivo BLE visto durante la scansione.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BleDispositivo {
    pub indirizzo: String,
    pub nome: String,
    pub rssi: Option<i16>,
    /// Il nome o i servizi annunciati fanno pensare a un adattatore OBD.
    pub probabile_obd: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GattServizio {
    pub uuid: String,
    /// (UUID, proprietà) di ogni caratteristica.
    pub caratteristiche: Vec<(String, String)>,
}

/// Servizi e caratteristiche scoperti sul dispositivo collegato.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GattInfo {
    pub nome: String,
    pub indirizzo: String,
    pub servizi: Vec<GattServizio>,
    /// Profilo documentato riconosciuto, se c'è.
    pub profilo: Option<String>,
    pub fonte_profilo: Option<String>,
}

impl GattInfo {
    /// Testo da copiare per la diagnosi di un profilo sconosciuto.
    pub fn testo(&self) -> String {
        let mut s = format!("Dispositivo BLE: {} [{}]\n", self.nome, self.indirizzo);
        match &self.profilo {
            Some(p) => s.push_str(&format!("Profilo riconosciuto: {p}\n")),
            None => s.push_str("Profilo riconosciuto: nessuno\n"),
        }
        for servizio in &self.servizi {
            s.push_str(&format!("Servizio {}\n", servizio.uuid));
            for (uuid, prop) in &servizio.caratteristiche {
                s.push_str(&format!("  Caratteristica {uuid}  [{prop}]\n"));
            }
        }
        s
    }
}

fn proprieta(f: CharPropFlags) -> String {
    let mut v = Vec::new();
    for (flag, nome) in [
        (CharPropFlags::READ, "lettura"),
        (CharPropFlags::WRITE, "scrittura"),
        (
            CharPropFlags::WRITE_WITHOUT_RESPONSE,
            "scrittura senza risposta",
        ),
        (CharPropFlags::NOTIFY, "notifica"),
        (CharPropFlags::INDICATE, "indicazione"),
    ] {
        if f.contains(flag) {
            v.push(nome);
        }
    }
    v.join(", ")
}

/// Scollega con un tempo massimo, ignorando gli errori.
async fn scollega_con_limite(p: &Peripheral, diario: &Diario) {
    match timeout(T_SCOLLEGAMENTO, p.disconnect()).await {
        Ok(Ok(())) => diario.passo("Scollegamento eseguito"),
        Ok(Err(e)) => diario.passo(format!("Scollegamento: {e}")),
        Err(_) => diario.passo(format!(
            "Scollegamento senza risposta entro {} s",
            T_SCOLLEGAMENTO.as_secs()
        )),
    }
}

/// Dispositivo collegato e servizi scoperti, prima di qualunque scrittura.
pub struct Scoperta {
    pub periferica: Peripheral,
    pub info: GattInfo,
    pub profilo: Option<&'static ProfiloBle>,
}

pub struct BleManager {
    rt: Arc<Runtime>,
    adapter: Adapter,
    diario: Diario,
}

impl BleManager {
    pub fn new(rt: Arc<Runtime>) -> Result<Self, BleErrore> {
        // ANDROID: prima di `Manager::new()` btleplug richiede
        // `btleplug::platform::init(&mut env)` con il `JNIEnv` della JVM
        // (da `android_activity`), la parte Java di btleplug nell'APK e i
        // permessi BLUETOOTH_SCAN/BLUETOOTH_CONNECT concessi a runtime.
        // Non implementato: vedi docs/ANDROID.md.
        let adapter = blocca(&rt, async {
            let manager = Manager::new()
                .await
                .map_err(|e| BleErrore::Altro(e.to_string()))?;
            let adattatori = manager.adapters().await.map_err(|e| match e {
                btleplug::Error::NoAdapterAvailable => BleErrore::NessunAdattatore,
                altro => BleErrore::Altro(altro.to_string()),
            })?;
            adattatori
                .into_iter()
                .next()
                .ok_or(BleErrore::NessunAdattatore)
        })??;
        Ok(BleManager {
            rt,
            adapter,
            diario: Diario::muto(),
        })
    }

    /// Dove annotare i passi (registro dei collegamenti, eco nell'app).
    pub fn con_diario(mut self, diario: Diario) -> Self {
        self.diario = diario;
        self
    }

    pub fn diario(&self) -> &Diario {
        &self.diario
    }

    /// Controlla che la radio Bluetooth sia accesa.
    pub fn verifica_radio(&self) -> Result<(), BleErrore> {
        match blocca(&self.rt, self.adapter.adapter_state())? {
            Ok(CentralState::PoweredOff) => Err(BleErrore::BluetoothSpento),
            Ok(_) => Ok(()),
            Err(e) => Err(BleErrore::Altro(e.to_string())),
        }
    }

    pub fn avvia_scansione(&self) -> Result<(), BleErrore> {
        if let Err(e) = self.verifica_radio() {
            self.diario
                .passo(format!("Scansione BLE non avviata: {}", e.testo()));
            return Err(e);
        }
        let esito = blocca(&self.rt, self.adapter.start_scan(ScanFilter::default()))?
            .map_err(|e| BleErrore::Altro(e.to_string()));
        match &esito {
            Ok(()) => self.diario.passo("Scansione BLE avviata (radio accesa)"),
            Err(e) => self
                .diario
                .passo(format!("Scansione BLE non avviata: {}", e.testo())),
        }
        esito
    }

    pub fn ferma_scansione(&self) {
        match blocca(&self.rt, self.adapter.stop_scan()) {
            Ok(Ok(())) => self.diario.passo("Scansione BLE fermata"),
            Ok(Err(e)) => self
                .diario
                .passo(format!("Arresto della scansione BLE: {e}")),
            Err(e) => self.diario.passo(e.testo()),
        }
    }

    /// Dispositivi visti finora, con i probabili adattatori OBD per primi.
    pub fn dispositivi(&self) -> Vec<BleDispositivo> {
        let Ok(mut out) = blocca(&self.rt, async {
            let mut out = Vec::new();
            for p in self.adapter.peripherals().await.unwrap_or_default() {
                let Ok(Some(prop)) = p.properties().await else {
                    continue;
                };
                let nome = prop
                    .local_name
                    .or(prop.advertisement_name)
                    .unwrap_or_default();
                let minuscolo = nome.to_lowercase();
                let per_nome = ["carista", "obd", "elm", "vlink", "v-link", "veepeak"]
                    .iter()
                    .any(|k| minuscolo.contains(k));
                let per_servizio = prop
                    .services
                    .iter()
                    .any(|s| PROFILI.iter().any(|pr| pr.servizio == *s));
                out.push(BleDispositivo {
                    indirizzo: p.address().to_string(),
                    nome,
                    rssi: prop.rssi,
                    probabile_obd: per_nome || per_servizio,
                });
            }
            out
        }) else {
            return Vec::new();
        };
        out.sort_by(|a, b| {
            b.probabile_obd
                .cmp(&a.probabile_obd)
                .then(a.nome.is_empty().cmp(&b.nome.is_empty()))
                .then(b.rssi.cmp(&a.rssi))
        });
        out
    }

    /// Su Windows collegarsi con l'osservatore degli annunci attivo è una
    /// causa nota di collegamenti instabili. La scansione viene fermata
    /// sempre, anche se il thread di lavoro l'ha già fermata (fermare un
    /// osservatore fermo non fa nulla), e la radio ha un attimo di respiro.
    fn ferma_scansione_per_collegare(&self) {
        match blocca(&self.rt, self.adapter.stop_scan()) {
            Ok(Ok(())) => self
                .diario
                .passo("Scansione BLE fermata prima del collegamento"),
            Ok(Err(e)) => self
                .diario
                .passo(format!("Arresto della scansione BLE: {e} (si prosegue)")),
            Err(e) => self.diario.passo(e.testo()),
        }
        std::thread::sleep(PAUSA_DOPO_SCANSIONE);
    }

    /// Collega il dispositivo e ne scopre servizi e caratteristiche, SENZA
    /// scrivere nulla su di esso. Usata dal collegamento e dalla diagnosi BLE.
    /// In caso di errore il dispositivo è già scollegato.
    pub fn collega_e_scopri(&self, indirizzo: &str) -> Result<Scoperta, BleErrore> {
        match self.verifica_radio() {
            Ok(()) => self.diario.passo("Radio Bluetooth: accesa"),
            Err(e) => {
                self.diario.passo(format!("Radio Bluetooth: {}", e.testo()));
                return Err(e);
            }
        }
        self.ferma_scansione_per_collegare();
        let d = &self.diario;
        blocca(&self.rt, async {
            let periferiche = self
                .adapter
                .peripherals()
                .await
                .map_err(|e| BleErrore::Altro(e.to_string()))?;
            let mut trovata = periferiche
                .into_iter()
                .find(|p| p.address().to_string() == indirizzo);
            // Dopo un tentativo fallito btleplug toglie il dispositivo
            // dall'elenco: su Windows lo si ritrova dal suo indirizzo.
            #[cfg(target_os = "windows")]
            if trovata.is_none()
                && let Ok(addr) = btleplug::api::BDAddr::from_str_delim(indirizzo)
            {
                d.passo("Dispositivo non più nell'elenco: lo si cerca per indirizzo");
                let id = btleplug::platform::PeripheralId::from(addr);
                trovata = self.adapter.add_peripheral(&id).await.ok();
            }
            let Some(periferica) = trovata else {
                d.passo("Dispositivo non più nell'elenco della scansione");
                return Err(BleErrore::NonRaggiungibile(
                    "dispositivo non più visibile: ripetere la scansione".into(),
                ));
            };
            let nome = match periferica.properties().await {
                Ok(Some(p)) => {
                    let nome = p
                        .local_name
                        .clone()
                        .or(p.advertisement_name.clone())
                        .unwrap_or_default();
                    let servizi: Vec<String> = p.services.iter().map(|s| s.to_string()).collect();
                    d.passo(format!(
                        "Dispositivo scelto: «{nome}» [{indirizzo}], RSSI {}, servizi annunciati: {}",
                        p.rssi
                            .map(|r| format!("{r} dBm"))
                            .unwrap_or_else(|| "sconosciuto".into()),
                        if servizi.is_empty() {
                            "nessuno".to_string()
                        } else {
                            servizi.join(", ")
                        }
                    ));
                    nome
                }
                _ => {
                    d.passo(format!(
                        "Dispositivo scelto: [{indirizzo}], proprietà non disponibili"
                    ));
                    String::new()
                }
            };

            // 1. Connessione radio: un solo nuovo tentativo, e solo per un
            //    errore transitorio.
            let mut tentativo = 1;
            loop {
                let t0 = Instant::now();
                d.passo(format!(
                    "Connessione radio: tentativo {tentativo} di 2 (limite {} s)",
                    T_COLLEGAMENTO.as_secs()
                ));
                let esito = timeout(T_COLLEGAMENTO, periferica.connect()).await;
                let ms = t0.elapsed().as_millis();
                let (causa, dettaglio) = match esito {
                    Ok(Ok(())) => {
                        d.passo(format!("Connessione radio riuscita in {ms} ms"));
                        break;
                    }
                    Ok(Err(e)) => (classifica(&e), e.to_string()),
                    Err(_) => (
                        CausaBle::TempoScaduto,
                        format!("nessuna risposta entro {} s", T_COLLEGAMENTO.as_secs()),
                    ),
                };
                d.passo(format!(
                    "Connessione radio fallita dopo {ms} ms: {dettaglio} [{}]",
                    causa.nome()
                ));
                scollega_con_limite(&periferica, d).await;
                if tentativo == 1 && causa.transitoria() {
                    d.passo(format!(
                        "Errore transitorio: nuovo tentativo fra {} ms",
                        PAUSA_PRIMA_DEL_RIPROVO.as_millis()
                    ));
                    tokio::time::sleep(PAUSA_PRIMA_DEL_RIPROVO).await;
                    tentativo += 1;
                    continue;
                }
                return Err(errore_di_fase(FaseBle::Collegamento, causa, &dettaglio));
            }

            // 2. Servizi e caratteristiche.
            let t0 = Instant::now();
            d.passo(format!(
                "Scoperta dei servizi GATT (limite {} s)",
                T_SCOPERTA.as_secs()
            ));
            let fallita = match timeout(T_SCOPERTA, periferica.discover_services()).await {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some((classifica(&e), e.to_string())),
                Err(_) => Some((
                    CausaBle::TempoScaduto,
                    format!("nessuna risposta entro {} s", T_SCOPERTA.as_secs()),
                )),
            };
            if let Some((causa, dettaglio)) = fallita {
                d.passo(format!(
                    "Scoperta dei servizi fallita dopo {} ms: {dettaglio} [{}]",
                    t0.elapsed().as_millis(),
                    causa.nome()
                ));
                scollega_con_limite(&periferica, d).await;
                return Err(errore_di_fase(FaseBle::ScopertaServizi, causa, &dettaglio));
            }
            let servizi = periferica.services();
            d.passo(format!(
                "Scoperta dei servizi riuscita in {} ms: {} servizi",
                t0.elapsed().as_millis(),
                servizi.len()
            ));

            let mut info = GattInfo {
                nome,
                indirizzo: indirizzo.to_string(),
                ..Default::default()
            };
            let mut viste = Vec::new();
            for servizio in servizi {
                d.passo(format!("  Servizio {}", servizio.uuid));
                let mut gs = GattServizio {
                    uuid: servizio.uuid.to_string(),
                    caratteristiche: Vec::new(),
                };
                for c in &servizio.characteristics {
                    d.passo(format!(
                        "    Caratteristica {}  [{}]",
                        c.uuid,
                        proprieta(c.properties)
                    ));
                    gs.caratteristiche
                        .push((c.uuid.to_string(), proprieta(c.properties)));
                    viste.push(CaratteristicaVista {
                        servizio: servizio.uuid,
                        uuid: c.uuid,
                        scrivibile: c.properties.intersects(
                            CharPropFlags::WRITE | CharPropFlags::WRITE_WITHOUT_RESPONSE,
                        ),
                        notificabile: c
                            .properties
                            .intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE),
                    });
                }
                info.servizi.push(gs);
            }
            let profilo = riconosci(&viste);
            match profilo {
                Some(p) => {
                    d.passo(format!("Profilo riconosciuto: {} ({})", p.nome, p.fonte));
                    info.profilo = Some(p.nome.to_string());
                    info.fonte_profilo = Some(p.fonte.to_string());
                }
                None => d.passo("Profilo riconosciuto: nessuno fra quelli documentati"),
            }
            d.passo(format!("MTU attuale: {}", periferica.mtu()));
            Ok(Scoperta {
                periferica,
                info,
                profilo,
            })
        })?
    }

    /// Scollega un dispositivo restituito da [`Self::collega_e_scopri`].
    pub fn scollega(&self, p: &Peripheral) {
        let _ = blocca(&self.rt, scollega_con_limite(p, &self.diario));
    }

    /// Collega il dispositivo, scopre i servizi e apre il canale seriale
    /// solo se il profilo è fra quelli documentati.
    pub fn collega(&self, indirizzo: &str) -> Result<(BleTransport, GattInfo), BleErrore> {
        let inizio = Instant::now();
        self.diario
            .passo(format!("=== Collegamento BLE a {indirizzo}: inizio ==="));
        let esito = self.collega_interno(indirizzo);
        let ms = inizio.elapsed().as_millis();
        match &esito {
            Ok((t, info)) => self.diario.passo(format!(
                "=== Collegamento BLE pronto in {ms} ms: {} ({}) ===",
                t.descrizione(),
                info.profilo.as_deref().unwrap_or("?")
            )),
            Err(e) => self.diario.passo(format!(
                "=== Collegamento BLE fallito dopo {ms} ms: {} ===",
                e.testo()
            )),
        }
        esito
    }

    fn collega_interno(&self, indirizzo: &str) -> Result<(BleTransport, GattInfo), BleErrore> {
        let s = self.collega_e_scopri(indirizzo)?;
        let Some(profilo) = s.profilo else {
            self.scollega(&s.periferica);
            return Err(BleErrore::ProfiloSconosciuto(s.info));
        };
        blocca(&self.rt, self.apri_canale(s.periferica, s.info, profilo))?
    }

    /// Notifiche e trasporto sul profilo riconosciuto. In caso di errore il
    /// dispositivo viene scollegato.
    async fn apri_canale(
        &self,
        periferica: Peripheral,
        info: GattInfo,
        profilo: &'static ProfiloBle,
    ) -> Result<(BleTransport, GattInfo), BleErrore> {
        let d = &self.diario;
        let trova = |uuid| {
            periferica
                .characteristics()
                .into_iter()
                .find(|c: &Characteristic| c.service_uuid == profilo.servizio && c.uuid == uuid)
        };
        let (Some(c_scrittura), Some(c_notifica)) =
            (trova(profilo.scrittura), trova(profilo.notifica))
        else {
            d.passo("Caratteristiche del profilo non trovate");
            scollega_con_limite(&periferica, d).await;
            return Err(BleErrore::ProfiloSconosciuto(info));
        };

        // 3. Flusso delle notifiche.
        let mut notifiche = match timeout(T_FLUSSO, periferica.notifications()).await {
            Ok(Ok(n)) => n,
            esito => {
                let (causa, dettaglio) = match esito {
                    Ok(Err(e)) => (classifica(&e), e.to_string()),
                    _ => (
                        CausaBle::TempoScaduto,
                        format!("nessuna risposta entro {} s", T_FLUSSO.as_secs()),
                    ),
                };
                d.passo(format!(
                    "Flusso delle notifiche non aperto: {dettaglio} [{}]",
                    causa.nome()
                ));
                scollega_con_limite(&periferica, d).await;
                return Err(errore_di_fase(FaseBle::FlussoNotifiche, causa, &dettaglio));
            }
        };

        // 4. Attivazione delle notifiche (scrittura del descrittore CCCD).
        let t0 = Instant::now();
        d.passo(format!(
            "Attivazione delle notifiche su {} (limite {} s)",
            c_notifica.uuid,
            T_ISCRIZIONE.as_secs()
        ));
        let fallita = match timeout(T_ISCRIZIONE, periferica.subscribe(&c_notifica)).await {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some((classifica(&e), e.to_string())),
            Err(_) => Some((
                CausaBle::TempoScaduto,
                format!("nessuna risposta entro {} s", T_ISCRIZIONE.as_secs()),
            )),
        };
        if let Some((causa, dettaglio)) = fallita {
            d.passo(format!(
                "Attivazione delle notifiche fallita dopo {} ms: {dettaglio} [{}]",
                t0.elapsed().as_millis(),
                causa.nome()
            ));
            scollega_con_limite(&periferica, d).await;
            return Err(errore_di_fase(
                FaseBle::AttivazioneNotifiche,
                causa,
                &dettaglio,
            ));
        }
        d.passo(format!(
            "Notifiche attive in {} ms; MTU {}",
            t0.elapsed().as_millis(),
            periferica.mtu()
        ));

        let caduto = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let uuid_notifica = c_notifica.uuid;
        let caduto_n = caduto.clone();
        let diario_n = d.clone();
        let t_notifiche = self.rt.spawn(async move {
            while let Some(n) = notifiche.next().await {
                if n.uuid == uuid_notifica && tx.send(n.value).is_err() {
                    return;
                }
            }
            diario_n.passo("Flusso delle notifiche chiuso da Windows: collegamento caduto");
            caduto_n.store(true, Ordering::Relaxed);
        });

        let mut tasks = vec![t_notifiche];
        if let Ok(mut eventi) = self.adapter.events().await {
            let id = periferica.id();
            let caduto_e = caduto.clone();
            let diario_e = d.clone();
            tasks.push(self.rt.spawn(async move {
                while let Some(ev) = eventi.next().await {
                    if let CentralEvent::DeviceDisconnected(chi) = ev
                        && chi == id
                    {
                        diario_e.passo(
                            "Disconnessione segnalata da Windows (evento DeviceDisconnected: \
                             dispositivo spento, fuori portata o collegamento chiuso)",
                        );
                        caduto_e.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }));
        }

        let tipo = if c_scrittura
            .properties
            .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
        {
            WriteType::WithoutResponse
        } else {
            WriteType::WithResponse
        };
        let indirizzo = info.indirizzo.clone();
        let trasporto = BleTransport {
            rt: self.rt.clone(),
            periferica,
            scrittura: c_scrittura,
            tipo,
            rx,
            caduto,
            tasks,
            nome: if info.nome.is_empty() {
                indirizzo
            } else {
                format!("{} [{indirizzo}]", info.nome)
            },
            chiuso: false,
            diario: d.clone(),
            caduta_annotata: false,
        };
        Ok((trasporto, info))
    }
}

pub struct BleTransport {
    rt: Arc<Runtime>,
    periferica: Peripheral,
    scrittura: Characteristic,
    tipo: WriteType,
    rx: mpsc::Receiver<Vec<u8>>,
    caduto: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
    nome: String,
    chiuso: bool,
    diario: Diario,
    caduta_annotata: bool,
}

impl BleTransport {
    fn verifica(&mut self) -> Result<(), Scollegato> {
        if self.chiuso || self.caduto.load(Ordering::Relaxed) {
            self.annota_caduta("collegamento Bluetooth interrotto");
            Err(Scollegato("collegamento Bluetooth interrotto".into()))
        } else {
            Ok(())
        }
    }

    /// Annota nel diario la prima caduta vista dal trasporto.
    fn annota_caduta(&mut self, motivo: &str) {
        if !self.caduta_annotata && !self.chiuso {
            self.caduta_annotata = true;
            self.diario.passo(format!("Trasporto BLE: {motivo}"));
        }
    }
}

impl Transport for BleTransport {
    fn write(&mut self, data: &[u8]) -> Result<(), Scollegato> {
        self.verifica()?;
        // 20 byte: carico utile garantito con l'MTU minima (23).
        for pezzo in data.chunks(20) {
            let esito = blocca(
                &self.rt,
                timeout(
                    Duration::from_secs(5),
                    self.periferica.write(&self.scrittura, pezzo, self.tipo),
                ),
            )
            .map_err(|e| Scollegato(e.testo()))?;
            let errore = match esito {
                Ok(Ok(())) => continue,
                Ok(Err(e)) => format!("scrittura BLE fallita: {e}"),
                Err(_) => "scrittura BLE senza risposta".to_string(),
            };
            self.annota_caduta(&errore);
            return Err(Scollegato(errore));
        }
        Ok(())
    }

    fn read(&mut self, attesa: Duration) -> Result<Vec<u8>, Scollegato> {
        let esito = if attesa.is_zero() {
            self.rx.try_recv().map_err(|e| match e {
                mpsc::TryRecvError::Empty => mpsc::RecvTimeoutError::Timeout,
                mpsc::TryRecvError::Disconnected => mpsc::RecvTimeoutError::Disconnected,
            })
        } else {
            self.rx.recv_timeout(attesa)
        };
        match esito {
            Ok(v) => Ok(v),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.verifica()?;
                Ok(Vec::new())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.annota_caduta("flusso delle notifiche terminato");
                Err(Scollegato("collegamento Bluetooth interrotto".into()))
            }
        }
    }

    fn descrizione(&self) -> String {
        format!("Bluetooth LE {}", self.nome)
    }

    /// Non blocca mai oltre [`T_SCOLLEGAMENTO`] e non va mai in panic. Se
    /// fosse chiamata dall'interno del runtime (non succede: il trasporto vive
    /// nel thread di lavoro) lo scollegamento partirebbe in background invece
    /// di usare `block_on`, che lì andrebbe in panic.
    fn close(&mut self) {
        if self.chiuso {
            return;
        }
        self.chiuso = true;
        for t in self.tasks.drain(..) {
            t.abort();
        }
        self.diario.passo("Scollegamento BLE richiesto");
        let p = self.periferica.clone();
        let diario = self.diario.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            self.rt
                .spawn(async move { scollega_con_limite(&p, &diario).await });
        } else {
            self.rt.block_on(scollega_con_limite(&p, &diario));
        }
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classificazione_degli_errori_winrt() {
        let casi = [
            (
                "Windows UWP threw error on subscribe: GattCommunicationStatus(3)",
                CausaBle::AccessoNegato,
            ),
            (
                "Error { code: HRESULT(0x80070005), message: \"Accesso negato.\" }",
                CausaBle::AccessoNegato,
            ),
            ("HRESULT(0x8065000F)", CausaBle::AccessoNegato),
            (
                "Windows UWP threw error on subscribe: GattCommunicationStatus(2)",
                CausaBle::ErroreProtocollo,
            ),
            (
                "The operation is not supported: ProtocolError",
                CausaBle::ErroreProtocollo,
            ),
            (
                "Windows UWP threw error on write: GattCommunicationStatus(1)",
                CausaBle::Irraggiungibile,
            ),
            ("HRESULT(0x800710DF)", CausaBle::Irraggiungibile),
            ("Timed out after 5s", CausaBle::TempoScaduto),
            ("Communication Error:", CausaBle::Altro),
        ];
        for (testo, atteso) in casi {
            assert_eq!(classifica_testo(testo), atteso, "{testo}");
        }
        assert_eq!(
            classifica(&btleplug::Error::PermissionDenied),
            CausaBle::AccessoNegato
        );
        assert_eq!(
            classifica(&btleplug::Error::NotConnected),
            CausaBle::Irraggiungibile
        );
        assert_eq!(
            classifica(&btleplug::Error::NotSupported("ProtocolError".into())),
            CausaBle::ErroreProtocollo
        );
        assert_eq!(
            classifica(&btleplug::Error::TimedOut(Duration::from_secs(1))),
            CausaBle::TempoScaduto
        );
    }

    #[test]
    fn solo_gli_errori_transitori_si_riprovano() {
        assert!(CausaBle::TempoScaduto.transitoria());
        assert!(CausaBle::Irraggiungibile.transitoria());
        assert!(!CausaBle::AccessoNegato.transitoria());
        assert!(!CausaBle::ErroreProtocollo.transitoria());
        assert!(!CausaBle::Altro.transitoria());
    }

    #[test]
    fn errori_con_il_passo_nel_testo() {
        match errore_di_fase(FaseBle::AttivazioneNotifiche, CausaBle::AccessoNegato, "x") {
            BleErrore::AccessoNegato(t) => {
                assert!(t.starts_with("attivazione delle notifiche: x"), "{t}")
            }
            altro => panic!("{altro:?}"),
        }
        match errore_di_fase(FaseBle::Collegamento, CausaBle::TempoScaduto, "y") {
            BleErrore::NonRaggiungibile(t) => assert!(t.starts_with("connessione radio: y")),
            altro => panic!("{altro:?}"),
        }
        // Ogni passo ha il suo nome e il suo limite.
        let fasi = [
            FaseBle::Collegamento,
            FaseBle::ScopertaServizi,
            FaseBle::FlussoNotifiche,
            FaseBle::AttivazioneNotifiche,
        ];
        for (i, a) in fasi.iter().enumerate() {
            assert!(a.limite() > Duration::ZERO);
            for b in &fasi[i + 1..] {
                assert_ne!(a.nome(), b.nome());
            }
        }
    }

    #[test]
    fn blocca_rifiuta_l_interno_del_runtime() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(blocca(&rt, async { 7 }).unwrap(), 7);
        let dentro = rt.block_on(async {
            let rt2 = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let esito = blocca(&rt2, async { 1 }).is_err();
            // Un runtime non si chiude dall'interno di un altro con `drop`.
            rt2.shutdown_background();
            esito
        });
        assert!(dentro);
    }
}
