# HoTGirl

Head tracker da smartphone per Linux/Wayland: uno smartphone fissato alla testa
fornisce l'orientamento (IMU a 100 Hz), la webcam del PC fungerà da riferimento
lento per posizione e yaw. Lo stesso nucleo software alimenta due uscite:

- **Giochi e simulatori:** pacchetti UDP verso [opentrack](https://github.com/opentrack/opentrack).
- **Desktop:** puntatore assoluto sul monitor (portal RemoteDesktop / libei su KDE Plasma).

Il cursore dipende solo dalla direzione del naso; lo sguardo degli occhi non conta.
Solo software, nessun hardware aggiuntivo.

- Progettazione completa: [docs/PLAN.md](docs/PLAN.md)
- Come funziona il sistema, con diagrammi: [docs/HOW_IT_WORKS.md](docs/HOW_IT_WORKS.md)

## Struttura

```
pc/                     Nucleo PC in Rust (workspace Cargo)
  hotgirl-protocol/     Formato dei pacchetti UDP telefono→PC e verso opentrack
  hotgirl-core/         Buffer campioni, sincronizzazione orologi, estrazione posa
  hotgirl-app/          Ricevitore UDP + finestra egui + invio opentrack
  spike-cursor/         Spike: cursore assoluto su Wayland (portal RemoteDesktop + libei)
android/                App Kotlin (foreground service, IMU a 100 Hz via UDP)
```

## Stato

### Implementato

- **Protocollo telefono→PC** (`hotgirl-protocol`): header 16 B (magic, versione, seq,
  timestamp), sample 44 B (timestamp sensore + quaternione + accuracy), ping/pong per
  la sincronizzazione orologi. Test di roundtrip.
- **Pacchetto opentrack**: 6 double little-endian (x, y, z, yaw, pitch, roll), porta 4242.
- **Nucleo** (`hotgirl-core`): stima offset orologi stile NTP (solo pong a RTT minimo),
  buffer campioni ordinato nel tempo (pronto per l'allineamento webcam), conversione
  quaternione → yaw/pitch/roll in convenzione opentrack. Test unitari.
- **App PC** (`hotgirl-app`): ricevitore UDP su porta 5555, finestra egui con posa live,
  statistiche e offset orologi, invio a opentrack attivabile dalla GUI.
- **Spike cursore** (`spike-cursor`): cursore assoluto via portal RemoteDesktop
  (`ConnectToEIS`) + libei, sweep dei 4 angoli di ogni monitor. Compila;
  **da validare su una sessione Plasma reale**.
- **App Android**: foreground service che invia `GAME_ROTATION_VECTOR` a 100 Hz via UDP
  e risponde ai ping; wake lock + WifiLock a bassa latenza; UI minima con IP/porta.
  **Mai compilata: serve un ambiente con SDK Android.**

### Da implementare (in ordine di priorità)

1. Compilare/testare l'app Android su telefono reale e misurare frequenza, perdita
   pacchetti, offset orologi.
2. Validare lo spike cursore su Plasma 6.8 (via libei; in alternativa portal+ScreenCast).
3. Logging e misure v1 solo IMU: jitter, deriva a testa ferma, sincronizzazione.
4. Webcam: posa della testa (X/Y/Z) e correzione della deriva dello yaw.
5. Calibrazione guidata a 9 punti con feedback visivo (LM, parametri: mount, yaw, offset mira).
6. Geometria monitor: intersezione raggio-piano, conversione in pixel, isteresi.
7. Multi-monitor con calibrazione PnP e raffinamento globale.
8. Autenticazione UDP (token), restore token del portal, interruttore di modalità.
9. Rifinitura: recentra per i giochi, filtro 1€, profili, robustezza Android.

## Sviluppo (PC)

```sh
cd pc
cargo test --workspace
cargo run -p hotgirl-app
cargo run -p spike-cursor   # richiede una sessione Wayland/KDE
```
