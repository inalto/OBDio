//! Bluetooth Low Energy tramite btleplug (API WinRT su Windows).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use btleplug::api::{
    Central, CentralEvent, CentralState, CharPropFlags, Characteristic, Manager as _,
    Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::profili::{CaratteristicaVista, PROFILI, riconosci};
use super::{Scollegato, Transport};

#[derive(Clone, Debug)]
pub enum BleErrore {
    /// Il PC non ha un adattatore Bluetooth utilizzabile.
    NessunAdattatore,
    /// Il Bluetooth è disattivato in Windows.
    BluetoothSpento,
    /// Il dispositivo scelto non risponde o rifiuta la connessione.
    NonRaggiungibile(String),
    /// Connesso, ma i servizi GATT non corrispondono a un profilo documentato.
    ProfiloSconosciuto(GattInfo),
    Altro(String),
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

pub struct BleManager {
    rt: Arc<Runtime>,
    adapter: Adapter,
}

impl BleManager {
    pub fn new(rt: Arc<Runtime>) -> Result<Self, BleErrore> {
        let adapter = rt.block_on(async {
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
        })?;
        Ok(BleManager { rt, adapter })
    }

    /// Controlla che la radio Bluetooth sia accesa.
    pub fn verifica_radio(&self) -> Result<(), BleErrore> {
        match self.rt.block_on(self.adapter.adapter_state()) {
            Ok(CentralState::PoweredOff) => Err(BleErrore::BluetoothSpento),
            Ok(_) => Ok(()),
            Err(e) => Err(BleErrore::Altro(e.to_string())),
        }
    }

    pub fn avvia_scansione(&self) -> Result<(), BleErrore> {
        self.verifica_radio()?;
        self.rt
            .block_on(self.adapter.start_scan(ScanFilter::default()))
            .map_err(|e| BleErrore::Altro(e.to_string()))
    }

    pub fn ferma_scansione(&self) {
        let _ = self.rt.block_on(self.adapter.stop_scan());
    }

    /// Dispositivi visti finora, con i probabili adattatori OBD per primi.
    pub fn dispositivi(&self) -> Vec<BleDispositivo> {
        let mut out = self.rt.block_on(async {
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
        });
        out.sort_by(|a, b| {
            b.probabile_obd
                .cmp(&a.probabile_obd)
                .then(a.nome.is_empty().cmp(&b.nome.is_empty()))
                .then(b.rssi.cmp(&a.rssi))
        });
        out
    }

    /// Collega il dispositivo, scopre i servizi e apre il canale seriale
    /// solo se il profilo è fra quelli documentati.
    pub fn collega(&self, indirizzo: &str) -> Result<(BleTransport, GattInfo), BleErrore> {
        self.verifica_radio()?;
        let rt = self.rt.clone();
        self.rt.block_on(async {
            let periferiche = self
                .adapter
                .peripherals()
                .await
                .map_err(|e| BleErrore::Altro(e.to_string()))?;
            let periferica = periferiche
                .into_iter()
                .find(|p| p.address().to_string() == indirizzo)
                .ok_or_else(|| {
                    BleErrore::NonRaggiungibile(
                        "dispositivo non più visibile: ripetere la scansione".into(),
                    )
                })?;
            let nome = match periferica.properties().await {
                Ok(Some(p)) => p.local_name.or(p.advertisement_name).unwrap_or_default(),
                _ => String::new(),
            };

            match timeout(Duration::from_secs(20), periferica.connect()).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(BleErrore::NonRaggiungibile(e.to_string())),
                Err(_) => {
                    return Err(BleErrore::NonRaggiungibile(
                        "nessuna risposta entro 20 secondi".into(),
                    ));
                }
            }
            match timeout(Duration::from_secs(20), periferica.discover_services()).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let _ = periferica.disconnect().await;
                    return Err(BleErrore::NonRaggiungibile(format!(
                        "scoperta dei servizi fallita: {e}"
                    )));
                }
                Err(_) => {
                    let _ = periferica.disconnect().await;
                    return Err(BleErrore::NonRaggiungibile(
                        "scoperta dei servizi senza risposta".into(),
                    ));
                }
            }

            let mut info = GattInfo {
                nome: nome.clone(),
                indirizzo: indirizzo.to_string(),
                ..Default::default()
            };
            let mut viste = Vec::new();
            for servizio in periferica.services() {
                let mut gs = GattServizio {
                    uuid: servizio.uuid.to_string(),
                    caratteristiche: Vec::new(),
                };
                for c in &servizio.characteristics {
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

            let Some(profilo) = riconosci(&viste) else {
                let _ = periferica.disconnect().await;
                return Err(BleErrore::ProfiloSconosciuto(info));
            };
            info.profilo = Some(profilo.nome.to_string());
            info.fonte_profilo = Some(profilo.fonte.to_string());

            let trova = |uuid| {
                periferica
                    .characteristics()
                    .into_iter()
                    .find(|c: &Characteristic| c.service_uuid == profilo.servizio && c.uuid == uuid)
            };
            let (Some(c_scrittura), Some(c_notifica)) =
                (trova(profilo.scrittura), trova(profilo.notifica))
            else {
                let _ = periferica.disconnect().await;
                return Err(BleErrore::ProfiloSconosciuto(info));
            };

            let mut notifiche = periferica
                .notifications()
                .await
                .map_err(|e| BleErrore::Altro(e.to_string()))?;
            if let Err(e) = periferica.subscribe(&c_notifica).await {
                let _ = periferica.disconnect().await;
                return Err(BleErrore::NonRaggiungibile(format!(
                    "attivazione delle notifiche fallita: {e}"
                )));
            }

            let caduto = Arc::new(AtomicBool::new(false));
            let (tx, rx) = mpsc::channel::<Vec<u8>>();
            let uuid_notifica = c_notifica.uuid;
            let caduto_n = caduto.clone();
            let t_notifiche = rt.spawn(async move {
                while let Some(n) = notifiche.next().await {
                    if n.uuid == uuid_notifica && tx.send(n.value).is_err() {
                        break;
                    }
                }
                caduto_n.store(true, Ordering::Relaxed);
            });

            let mut tasks = vec![t_notifiche];
            if let Ok(mut eventi) = self.adapter.events().await {
                let id = periferica.id();
                let caduto_e = caduto.clone();
                tasks.push(rt.spawn(async move {
                    while let Some(ev) = eventi.next().await {
                        if let CentralEvent::DeviceDisconnected(chi) = ev
                            && chi == id
                        {
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
            let trasporto = BleTransport {
                rt: rt.clone(),
                periferica,
                scrittura: c_scrittura,
                tipo,
                rx,
                caduto,
                tasks,
                nome: if nome.is_empty() {
                    indirizzo.to_string()
                } else {
                    format!("{nome} [{indirizzo}]")
                },
                chiuso: false,
            };
            Ok((trasporto, info))
        })
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
}

impl BleTransport {
    fn verifica(&self) -> Result<(), Scollegato> {
        if self.chiuso || self.caduto.load(Ordering::Relaxed) {
            Err(Scollegato("collegamento Bluetooth interrotto".into()))
        } else {
            Ok(())
        }
    }
}

impl Transport for BleTransport {
    fn write(&mut self, data: &[u8]) -> Result<(), Scollegato> {
        self.verifica()?;
        // 20 byte: carico utile garantito con l'MTU minima (23).
        for pezzo in data.chunks(20) {
            let esito = self.rt.block_on(timeout(
                Duration::from_secs(5),
                self.periferica.write(&self.scrittura, pezzo, self.tipo),
            ));
            match esito {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(Scollegato(format!("scrittura BLE fallita: {e}"))),
                Err(_) => return Err(Scollegato("scrittura BLE senza risposta".into())),
            }
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
                Err(Scollegato("collegamento Bluetooth interrotto".into()))
            }
        }
    }

    fn descrizione(&self) -> String {
        format!("Bluetooth LE {}", self.nome)
    }

    fn close(&mut self) {
        if self.chiuso {
            return;
        }
        self.chiuso = true;
        for t in self.tasks.drain(..) {
            t.abort();
        }
        let _ = self.rt.block_on(timeout(
            Duration::from_secs(3),
            self.periferica.disconnect(),
        ));
    }
}

impl Drop for BleTransport {
    fn drop(&mut self) {
        self.close();
    }
}
