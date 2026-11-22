//! Parti condivise dalle prove d'integrazione: il broker MQTT di prova in
//! questo stesso processo e l'attesa degli eventi del thread di lavoro.

#![allow(dead_code)]

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use obdio::worker::Evento;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Broker di prova
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Will {
    pub argomento: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub trattenuto: bool,
}

#[derive(Clone, Debug)]
pub struct Connect {
    pub client_id: String,
    pub pulita: bool,
    pub utente: Option<String>,
    pub password: Option<String>,
    pub will: Option<Will>,
}

#[derive(Clone, Debug)]
pub struct Pubblicazione {
    pub argomento: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub trattenuto: bool,
}

impl Pubblicazione {
    pub fn testo(&self) -> String {
        String::from_utf8_lossy(&self.payload).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.payload).expect("payload non è JSON")
    }
}

#[derive(Default)]
pub struct Registro {
    pub connect: Vec<Connect>,
    pub pubblicazioni: Vec<Pubblicazione>,
    pub disconnect: usize,
}

pub type Condiviso = Arc<(Mutex<Registro>, Condvar)>;

pub struct Broker {
    pub porta: u16,
    pub registro: Condiviso,
    pub stop: Arc<AtomicBool>,
    pub flussi: Arc<Mutex<Vec<TcpStream>>>,
    pub accettazione: Option<JoinHandle<()>>,
}

pub fn stringa(corpo: &[u8], i: &mut usize) -> Vec<u8> {
    let n = u16::from_be_bytes([corpo[*i], corpo[*i + 1]]) as usize;
    let v = corpo[*i + 2..*i + 2 + n].to_vec();
    *i += 2 + n;
    v
}

pub fn leggi_pacchetto(s: &mut TcpStream) -> io::Result<(u8, Vec<u8>)> {
    let mut b = [0u8; 1];
    s.read_exact(&mut b)?;
    let primo = b[0];
    let (mut lunghezza, mut moltiplicatore) = (0usize, 1usize);
    loop {
        s.read_exact(&mut b)?;
        lunghezza += (b[0] & 0x7F) as usize * moltiplicatore;
        if b[0] & 0x80 == 0 {
            break;
        }
        moltiplicatore *= 128;
    }
    let mut corpo = vec![0u8; lunghezza];
    s.read_exact(&mut corpo)?;
    Ok((primo, corpo))
}

pub fn gestisci_connessione(mut s: TcpStream, registro: Condiviso) {
    let _ = s.set_nodelay(true);
    while let Ok((primo, corpo)) = leggi_pacchetto(&mut s) {
        match primo >> 4 {
            1 => {
                let mut i = 0;
                let _nome = stringa(&corpo, &mut i);
                let flag = corpo[i + 1];
                i += 4; // livello, flag, keep alive
                let client_id = String::from_utf8(stringa(&corpo, &mut i)).unwrap();
                let will = (flag & 0x04 != 0).then(|| Will {
                    argomento: String::from_utf8(stringa(&corpo, &mut i)).unwrap(),
                    payload: stringa(&corpo, &mut i),
                    qos: (flag >> 3) & 3,
                    trattenuto: flag & 0x20 != 0,
                });
                let utente =
                    (flag & 0x80 != 0).then(|| String::from_utf8(stringa(&corpo, &mut i)).unwrap());
                let password =
                    (flag & 0x40 != 0).then(|| String::from_utf8(stringa(&corpo, &mut i)).unwrap());
                let c = Connect {
                    client_id,
                    pulita: flag & 0x02 != 0,
                    utente,
                    password,
                    will,
                };
                // Prima la risposta, poi la registrazione: chi attende il
                // CONNECT sa che il client ha già il suo CONNACK.
                if s.write_all(&[0x20, 0x02, 0x00, 0x00]).is_err() {
                    break;
                }
                let (m, cv) = &*registro;
                m.lock().unwrap().connect.push(c);
                cv.notify_all();
            }
            3 => {
                let qos = (primo >> 1) & 3;
                let mut i = 0;
                let argomento = String::from_utf8(stringa(&corpo, &mut i)).unwrap();
                let mut pid = [0u8; 2];
                if qos > 0 {
                    pid = [corpo[i], corpo[i + 1]];
                    i += 2;
                }
                let p = Pubblicazione {
                    argomento,
                    payload: corpo[i..].to_vec(),
                    qos,
                    trattenuto: primo & 1 != 0,
                };
                if qos == 1 && s.write_all(&[0x40, 0x02, pid[0], pid[1]]).is_err() {
                    break;
                }
                let (m, cv) = &*registro;
                m.lock().unwrap().pubblicazioni.push(p);
                cv.notify_all();
            }
            12 => {
                if s.write_all(&[0xD0, 0x00]).is_err() {
                    break;
                }
            }
            14 => {
                let (m, cv) = &*registro;
                m.lock().unwrap().disconnect += 1;
                cv.notify_all();
                break;
            }
            _ => {}
        }
    }
}

impl Broker {
    /// In ascolto su `porta` (0: una libera). Riprova a lungo se la porta è
    /// appena stata liberata da un altro broker.
    pub fn avvia(porta: u16) -> Broker {
        let scadenza = Instant::now() + Duration::from_secs(10);
        let ascolto = loop {
            match TcpListener::bind(("127.0.0.1", porta)) {
                Ok(l) => break l,
                Err(e) if Instant::now() < scadenza => {
                    let _ = e;
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => panic!("porta {porta} non disponibile: {e}"),
            }
        };
        let porta = ascolto.local_addr().unwrap().port();
        ascolto.set_nonblocking(true).unwrap();
        let registro: Condiviso = Arc::new((Mutex::new(Registro::default()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let flussi = Arc::new(Mutex::new(Vec::new()));
        let (r, s, f) = (registro.clone(), stop.clone(), flussi.clone());
        let accettazione = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                match ascolto.accept() {
                    Ok((flusso, _)) => {
                        flusso.set_nonblocking(false).unwrap();
                        f.lock().unwrap().push(flusso.try_clone().unwrap());
                        let r = r.clone();
                        std::thread::spawn(move || gestisci_connessione(flusso, r));
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
            // `ascolto` esce di scope qui: la porta si libera.
        });
        Broker {
            porta,
            registro,
            stop,
            flussi,
            accettazione: Some(accettazione),
        }
    }

    /// Spegne il broker: chiude le connessioni aperte e libera la porta.
    pub fn ferma(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for f in self.flussi.lock().unwrap().drain(..) {
            let _ = f.shutdown(Shutdown::Both);
        }
        if let Some(h) = self.accettazione.take() {
            let _ = h.join();
        }
    }

    /// Attende che `f` trovi qualcosa nel registro.
    pub fn attendi<T>(&self, secondi: u64, mut f: impl FnMut(&Registro) -> Option<T>) -> T {
        let (m, cv) = &*self.registro;
        let scadenza = Instant::now() + Duration::from_secs(secondi);
        let mut r = m.lock().unwrap();
        loop {
            if let Some(x) = f(&r) {
                return x;
            }
            let resto = scadenza
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| {
                    panic!(
                        "il broker non ha ricevuto quanto atteso; pubblicazioni viste: {:?}",
                        r.pubblicazioni
                            .iter()
                            .map(|p| p.argomento.as_str())
                            .collect::<std::collections::BTreeSet<_>>()
                    )
                });
            r = cv
                .wait_timeout(r, resto.min(Duration::from_millis(200)))
                .unwrap()
                .0;
        }
    }

    /// Prima pubblicazione su `argomento` per cui `f` è vero.
    pub fn pubblicazione(
        &self,
        secondi: u64,
        argomento: &str,
        f: impl Fn(&Pubblicazione) -> bool,
    ) -> Pubblicazione {
        self.attendi(secondi, |r| {
            r.pubblicazioni
                .iter()
                .find(|p| p.argomento == argomento && f(p))
                .cloned()
        })
    }

    pub fn tutte(&self) -> Vec<Pubblicazione> {
        self.registro.0.lock().unwrap().pubblicazioni.clone()
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.ferma();
    }
}

/// Attende il primo evento per cui `f` restituisce un valore.
pub fn attendi<T>(
    rx: &Receiver<Evento>,
    secondi: u64,
    mut f: impl FnMut(Evento) -> Option<T>,
) -> T {
    let scadenza = Instant::now() + Duration::from_secs(secondi);
    loop {
        let resto = scadenza
            .checked_duration_since(Instant::now())
            .expect("evento atteso non arrivato in tempo");
        let ev = rx
            .recv_timeout(resto)
            .expect("evento atteso non arrivato in tempo");
        if let Some(x) = f(ev) {
            return x;
        }
    }
}
