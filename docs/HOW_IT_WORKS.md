# Come funziona HoTGirl

Documento illustrativo dell'architettura e del flusso dei dati.
I diagrammi sono in [Mermaid](https://mermaid.js.org/) e vengono renderizzati da GitHub.
Per le decisioni di progetto e i rischi: [PLAN.md](PLAN.md).

## 1. Panoramica

Un unico nucleo software produce la posa della testa; da lì partono due uscite.
Lo stato attuale è la **v1 solo IMU** (niente webcam ancora): le parti grigie/dashed
non sono implementate.

```mermaid
flowchart LR
    subgraph phone["📱 Pixel 9a (Kotlin)"]
        S["GAME_ROTATION_VECTOR<br/>100 Hz"]
        APP["HeadTrackerService<br/>foreground service<br/>wake lock + WifiLock"]
        S --> APP
    end

    APP -- "UDP 44 B @ 100 Hz<br/>quaternione + timestamp" --> RX

    subgraph pc["💻 PC (Rust)"]
        RX["ricevitore UDP<br/>porta 5555"]
        SYNC["stima offset orologi<br/>(ping/pong, min RTT)"]
        CORE["nucleo: buffer campioni<br/>+ quaternione → yaw/pitch/roll"]
        GUI["finestra egui:<br/>posa live, statistiche"]
        RX --> CORE
        SYNC --> CORE
        RX --> GUI
    end

    APP <-. "ping/pong<br/>ogni 200 ms" .-> SYNC

    CORE -- "6 double LE<br/>porta 4242" --> OT["opentrack<br/>(giochi)"]
    CORE -.-> WEB["webcam: X/Y/Z<br/>+ yaw assoluto<br/>(da implementare)"]
    CORE -.-> CUR["cursore assoluto<br/>su Wayland<br/>(da implementare)"]
```

## 2. Il viaggio di un campione

Ogni evento del sensore diventa un pacchetto UDP che attraversa tutto il sistema:

```mermaid
sequenceDiagram
    participant HW as Sensore (IMU)
    participant PH as App Android
    participant PC as Nucleo Rust
    participant OT as opentrack

    HW->>PH: evento a 100 Hz<br/>(timestamp sensore)
    PH->>PC: UDP sample (44 byte)<br/>quaternione + event_ts + sent_ts
    PC->>PC: decodifica (magic, versione)
    PC->>PC: event_ts convertito al clock PC<br/>tramite offset stimato
    PC->>PC: quaternione → yaw/pitch/roll
    PC->>OT: UDP 48 byte (6 double LE)<br/>se "invia a opentrack" è attivo
```

### Formato dei pacchetti telefono→PC

Tutti i campi little-endian. Header comune di 16 byte, poi il payload in base al tipo.

```
Header (16 byte)                    Sample (28 byte)                Pong (16 byte)
┌──────────┬──────────────┐        ┌──────────────┬───────────┐   ┌──────────────┬─────────────┐
│ magic 4B │ "HTGR"       │        │ event_ts 8B  │ ns sensore│   │ t_pc 8B      │ eco del ping│
│ version1B│ 1            │        │ quat_x  4B   │ f32       │   │ t_phone 8B   │ clock phone │
│ kind   1B│ 1=2=3        │        │ quat_y  4B   │ f32       │   └──────────────┴─────────────┘
│ seq    2B│ contatore    │        │ quat_z  4B   │ f32       │
│ sent_ts8B│ clock phone  │        │ quat_w  4B   │ f32       │   Ping (8 byte, PC→phone)
└──────────┴──────────────┘        │ accuracy 4B  │ f32       │   ┌──────────────┐
                                   └──────────────┴───────────┘   │ t_pc 8B      │
```

## 3. Sincronizzazione degli orologi

Il telefono e il PC hanno orologi diversi. Il PC misura l'offset con uno
scambio ping-pong stile NTP e si fida solo delle risposte più veloci,
perché quelle sono le meno distorte dall'attesa in coda:

```mermaid
sequenceDiagram
    participant PC
    participant PH as Telefono

    PC->>PH: ping (t_pc = clock PC)
    PH-->>PC: pong (t_pc eco, t_phone = clock telefono)
    Note over PC: rtt = ricezione − t_pc<br/>offset ≈ t_phone − (t_pc + ricezione)/2
    Note over PC: l'offset usato viene dalla media<br/>dei pong con rtt ≤ migliore + 1 ms
```

L'offset serve a convertire `event_ts` (clock telefono) nel clock del PC, così i
campioni IMU possono essere allineati agli istanti esatti — necessario quando
arriverà la webcam a 5-10 Hz sopra il flusso IMU a 100 Hz.

## 4. Dalla rotazione alla posa

Il sensore `GAME_ROTATION_VECTOR` dà un quaternione (x, y, z, w) che rappresenta
l'orientamento del telefono rispetto al mondo. Il nucleo lo scompone in
yaw/pitch/roll con la convenzione di opentrack:

```mermaid
flowchart LR
    Q["quaternione<br/>(x, y, z, w)"] --> M["matrice di rotazione<br/>R = Ry(yaw)·Rx(pitch)·Rz(roll)"]
    M --> Y["yaw = atan2(R₀₂, R₂₂)<br/>pitch = asin(−R₁₂)<br/>roll = atan2(R₁₀, R₁₁)"]
    Y --> P["posa (gradi)<br/>+ x/y/z (oggi: 0)"]
    P --> OT["pacchetto opentrack:<br/>6 × f64 LE su porta 4242"]
```

**Frame di riferimento:** X destra, Y su, Z verso l'utente. Yaw ruota attorno
all'asse verticale (guarda a destra/sinistra), pitch su/giù, roll inclinazione laterale.

## 5. Uscita giochi: opentrack

Il nucleo non parla direttamente coi giochi: spedisce la posa grezza a opentrack,
che si occupa di curve, filtri e protocolli verso i giochi (glue Wine/freetrack,
joystick virtuale, plugin vari). In questa modalità la geometria dei monitor
non conta: servono 6DOF e un "recentra" (da implementare).

## 6. Spike cursore: come funziona la via libei

Su Wayland non esiste un'API per posizionare il cursore in modo assoluto.
Lo spike (`pc/crates/spike-cursor`) prova la via più recente: il portal
RemoteDesktop di KDE che espone un socket libei, con `reis` come client EI in Rust.

```mermaid
sequenceDiagram
    participant SP as spike-cursor
    participant PT as xdg-desktop-portal (KDE)
    participant KW as KWin

    SP->>PT: CreateSession + SelectDevices(Pointer)
    Note over SP,PT: dialoghi di consenso utente
    SP->>PT: Start + ConnectToEIS → fd socket
    SP->>KW: handshake EI (context_type = Sender)
    KW-->>SP: Seat → bind(PointerAbsolute)
    KW-->>SP: Device (con regioni = monitor) → Resumed
    SP->>KW: motion_absolute(x, y) + frame<br/>sweep sui 4 angoli di ogni regione
```

Le **regioni** del device EI corrispondono ai monitor (offset e dimensioni in
pixel logici): sono la base naturale per il futuro puntamento multi-monitor.
Se questa via non funziona su Plasma 6.8, l'alternativa è portal RemoteDesktop +
ScreenCast con `NotifyPointerMotionAbsolute` (via 1 del piano).

## 7. App Android

```mermaid
flowchart TB
    UI["MainActivity:<br/>IP + porta, Avvia/Ferma"] -->|intent extras| SVC
    subgraph SVC["HeadTrackerService (foreground)"]
        N["notifica persistente"]
        WL["PARTIAL_WAKE_LOCK<br/>+ WifiLock LOW_LATENCY"]
        SEN["SensorListener<br/>GAME_ROTATION_VECTOR, 10 ms"]
        UDP["socket UDP<br/>invio sample + risposta ping"]
        SEN --> UDP
    end
```

Il servizio parte con `startForegroundService`, quindi sopravvive alla chiusura
dell'UI; con lo schermo spento i wake lock tengono attivi sensore e Wi-Fi.
Da verificare sul telefono reale: frequenza effettiva del sensore, comportamento
con l'ottimizzazione batteria, latenza Wi-Fi.

## 8. Cosa manca a una foto

Mappa rapida delle parti che completano il disegno (dettagli in [PLAN.md](PLAN.md)):

```mermaid
flowchart LR
    subgraph now["Oggi (v1 solo IMU)"]
        A["IMU 100 Hz<br/>→ yaw/pitch/roll<br/>→ opentrack"]
    end
    subgraph next["Prossimi passi"]
        B["webcam:<br/>posa testa + yaw assoluto"]
        C["calibrazione 9 punti<br/>(mount, yaw, offset mira)"]
        D["geometria monitor:<br/>raggio → piano → pixel"]
    end
    subgraph later["Dopo"]
        E["multi-monitor<br/>con PnP"]
        F["cursore assoluto<br/>+ modalità desktop"]
        G["token UDP,<br/>profili, filtri"]
    end
    A --> B --> C --> D --> E --> F --> G
```
