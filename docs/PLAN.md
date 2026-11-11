# Smartphone Head Tracker – Riepilogo per revisione

Versione 0.2 · 11 novembre 2026 · Bozza di progetto per raccogliere opinioni. Stato: solo progettazione, nessuna implementazione e nessuna misura ancora.

## 1. Obiettivo in breve

Usare uno smartphone fissato alla testa come sensore di orientamento, insieme alla webcam del PC, per due scopi con lo stesso nucleo software:

- **Giochi e simulatori:** inviare la posa della testa (rotazione e posizione) a opentrack, che la trasforma in input per il gioco.
- **Desktop:** guardare un punto del monitor e far andare il cursore esattamente lì, con un piccolo errore accettato, anche con più monitor posizionati in modo diverso nello spazio.

Principio fondamentale: il cursore dipende **solo dalla direzione in cui punta il naso**. Lo sguardo degli occhi non deve contare nulla, così si può guardare intorno con gli occhi senza spostare il cursore.

Vincolo di progetto: solo software, usando solo ciò che si ha già (un PC e un telefono più la webcam già collegata), con il minor numero possibile di componenti e il minimo consumo di risorse, per poter giocare mentre il sistema gira.

## 2. Ambiente di sviluppo

- Telefono: Pixel 9a, non rootato, aggiornato.
- PC: CachyOS (base Arch), aggiornato, con KDE Plasma 6.8 su Wayland. Webcam collegata.
- Linguaggi: Rust sul PC (scelta presa, niente Python); Kotlin per l'app Android.
- Sviluppo iniziale solo per Linux/Wayland.

## 3. Architettura

Un nucleo unico produce la posa della testa; da lì partono due uscite.

- **App Android (Kotlin):** foreground service con wake lock che legge `GAME_ROTATION_VECTOR`, mette il timestamp dell'evento e invia i campioni a circa 100 Hz. Risponde anche ai ping per la sincronizzazione degli orologi.
- **Trasporto:** UDP su Wi-Fi (proposto per la prima fase perché più facile da debuggare) oppure Bluetooth Low Energy (preferito nella specifica originale). Decisione aperta.
- **Nucleo PC (Rust):** riceve i campioni, stima l'offset tra gli orologi, applica fusione e filtri, calcola la posa. Librerie previste: `nalgebra`, `egui/eframe` per la GUI, `std::net::UdpSocket` senza runtime asincrono all'inizio.
- **Webcam:** serve per la posizione della testa (X/Y/Z) e come riferimento lento dello yaw per correggere la deriva dell'IMU. Non serve per gli occhi.
- **Uscita giochi:** pacchetto UDP verso opentrack con sei double little-endian (x, y, z, yaw, pitch, roll), porta 4242 di default.
- **Uscita desktop:** puntatore assoluto tramite portal RemoteDesktop e libei su KDE Plasma.
- **GUI:** calibrazione guidata, stato dei sensori, diagnostica e interruttore di modalità (giochi / desktop / pausa).

## 4. Matematica e correzione degli errori

**Raggio e piano.** Si costruisce un raggio con origine nella posizione della testa e direzione data dall'orientamento dell'IMU corretto: `d = R_mondo←IMU · R_mount · f`, origine `p = pos_webcam + R·o_testa`. L'intersezione con il piano del monitor (normale n, centro c) è `t = n·(c−p)/(n·d)`, valida se `t>0`, e il punto è `p + t·d`. Poi si converte in coordinate fisiche e in pixel.

**Calibrazione.** Parametri da stimare, circa 7: la rotazione fissa telefono→testa `R_mount` (3), lo yaw dell'IMU rispetto al monitor (1) e l'offset del punto di mira rispetto alla testa (3). Con 9 punti ci sono 18 equazioni; si risolve con minimi quadrati non lineari (Levenberg–Marquardt) con perdita robusta, tenendo da parte punti di validazione. Idea chiave: mostrare in tempo reale il punto stimato e far muovere la testa finché è sul bersaglio. Così la mira del naso è definita dall'utente con feedback visivo e non da un modello astratto.

**Deriva dello yaw.** Il giroscopio non ha un riferimento assoluto per lo yaw. Proposta: filtro di Kalman scalare con stato = bias dello yaw (random walk) e misura = yaw della webcam meno yaw dell'IMU, attivo solo quando la faccia è affidabile, la testa lenta e l'angolo non estremo. Una correzione lenta (decine di secondi) è quasi invisibile perché l'utente compensa muovendo la testa.

**Tempi.** I timestamp Android usano la base `elapsedRealtimeNanos` e indicano il momento dell'evento. Il PC ha un orologio diverso: serve uno scambio ping-pong periodico (stile NTP, tenendo il round trip più breve). Le misure della webcam vanno applicate al campione IMU del loro istante di cattura, usando un buffer storico.

**Filtraggio.** Filtro 1€ (Casiez et al., 2012) sul punto finale: poco jitter da fermo, poca latenza in movimento. Isteresi tra monitor adiacenti.

**Budget d'errore (geometria).** Un grado a 60 cm vale circa 1 cm sullo schermo. Un errore laterale di 3 cm nella posizione della testa sposta il punto di circa 3 cm. L'errore in profondità conta meno quando si guarda dritto. Per questo X/Y della webcam sono più importanti di Z.

**Più monitor nello spazio.** Ogni monitor è un rettangolo con centro, assi, normale e dimensioni fisiche (da EDID). Il raggio si interseca con tutti i piani e si sceglie il `t>0` più piccolo dentro il rettangolo. Per trovare la posa di un monitor senza misurarla: dopo aver calibrato il primo monitor, si punta il naso sui 4 angoli di ogni altro monitor da fermi; sono 4 direzioni verso i vertici di un rettangolo di dimensioni note, cioè un problema PnP che restituisce posa e scala. Poi un raffinamento globale di tutti i parametri. Da gestire: raggio fuori da tutti i monitor (tenere l'ultimo con isteresi), monitor molto inclinati (l'errore cresce con 1/cos dell'angolo), monitor curvi o ultrawide (spezzare in 2–3 piani).

## 5. Cursore su Wayland / KDE Plasma

Su Wayland non c'è un'API standard per posizionare il cursore in modo assoluto. Tool come ydotool sono relativi e distorti dall'accelerazione, e un dispositivo uinput assoluto mappa un solo output. Le strade valutate:

1. **Portal RemoteDesktop con `NotifyPointerMotionAbsolute` + ScreenCast** (uno stream per monitor, coordinate logiche relative allo stream). È la via più collaudata su Plasma 6; richiede il dialogo ScreenCast.
2. **Portal RemoteDesktop + `ConnectToEIS` e libei** con dispositivo assoluto e regioni dei monitor: nessun secondo dialogo. Su KDE risulta meno verificata (un progetto la dichiara implementata ma non verificata su una sessione Plasma reale).
3. **Interfaccia EIS privata di KWin**, che salta il dialogo: pensata per sessioni virtuali isolate, non affidabile per il desktop reale.

Il consenso va chiesto una volta e ricordato con un restore token. Crate Rust: `ashpd` (portal) e `reis` (libei in Rust puro). Le fonti parlano di Plasma 6 in generale: nulla di specifico è stato trovato su Plasma 6.8, quindi il primo test va fatto direttamente sul sistema.

## 6. Giochi con opentrack

Il nucleo invia i pacchetti a opentrack, che si occupa di curve, filtri e protocolli di uscita (glue Wine/freetrack, joystick virtuale, plugin X-Plane, ecc.). Per i giochi Proton c'è un vincolo: opentrack e il gioco devono stare nello stesso Wine prefix. Il comportamento dipende dal singolo gioco e va provato presto. In questa modalità servono posa 6DOF e un semplice «ricentra», non la geometria dei monitor.

## 7. Risorse e prestazioni

- IMU a 100 Hz via UDP: costo trascurabile sul PC.
- Webcam: la parte più pesante; 5–10 Hz con immagini piccole (per esempio 320×240) e priorità bassa; tra un frame e l'altro si interpola con l'IMU.
- Telefono con schermo spento e foreground service; `WifiLock` a bassa latenza per evitare il risparmio energetico del Wi-Fi.
- Pochi thread, nessun runtime pesante, niente garbage collector sul PC.
- L'accelerometro non va usato per la posizione (l'errore cresce col quadrato del tempo): solo per brevi interpolazioni (100–200 ms) o come fallback di circa un secondo se la faccia si perde.

## 8. Piano di sviluppo

1. **Spike del cursore** (rischio maggiore): piccolo programma Rust che sposta il cursore sugli angoli di ogni monitor con le vie 1 e 2 della sezione 5.
2. **v1 solo IMU:** app Kotlin → UDP → Rust; finestra egui con il rettangolo del monitor e il punto stimato; registro dei dati; misure di jitter, perdita pacchetti e deriva (testa ferma 5–10 minuti).
3. **Uscita opentrack** con «ricentra».
4. **Webcam:** posizione X/Y/Z e yaw assoluto; correzione della deriva.
5. **Calibrazione guidata** (5 o 9 punti) e puntamento assoluto su un monitor.
6. **Multi-monitor** con calibrazione PnP e raffinamento globale.
7. **Rifinitura:** interruttore di modalità, profili, filtri, test sul campo.

Metriche: errore medio e massimo in pixel su punti non usati in calibrazione; ripetibilità tra sessioni; deriva a testa ferma; errore con traslazioni; latenza end-to-end; percentuale di frame webcam validi; comportamento se la faccia o il Wi-Fi si perdono.

## 9. Decisioni prese, proposte e aperte

**Prese:** Rust sul PC; telefono Pixel 9a con app Kotlin; solo software e hardware già posseduto; Linux/Wayland/KDE per iniziare; gli occhi non influenzano il cursore; supporto sia ai giochi sia al desktop; la webcam c'è.

**Proposte, non ancora confermate:** UDP per la fase iniziale; filtro di Kalman scalare per lo yaw; calibrazione a 9 punti con feedback visivo; PnP per i monitor aggiuntivi; opentrack come uscita per i giochi.

## 10. Dubbi, rischi e punti mancanti

- **Come si fa clic?** Non è definito. Opzioni: mouse fisico per i clic, tempo di permanenza (dwell), tasto/gesto. Senza questo il puntatore è incompleto.
- **Precisione realistica:** la stima di 1–3° dopo calibrazione è una previsione, non una misura. In uno studio con IMU negli auricolari (hardware diverso) la deriva dello yaw era grande anche con il magnetometro; per un tracker da camera RGB l'errore riportato era intorno ai 7,6°. Entrambi i risultati vanno rimisurati sul Pixel 9a.
- **Cursore assoluto su KDE 6.8:** da verificare sul sistema reale; la via libei con regioni è la meno verificata.
- **Webcam in Rust:** per quanto si sappia non esistono binding Rust ufficiali di MediaPipe; probabili strade sono modelli ONNX con `ort` oppure OpenCV, e la posa va forse calcolata con `solvePnP` dai landmark.
- **Fissaggio e comfort:** un telefono slittato sposta la mira; peso e calore sulla testa; serve un supporto rigido e ripetibile (fascia).
- **Ergonomia:** con mappatura 1:1 per raggiungere gli angoli serve ruotare il collo, e su più monitor affiancati l'angolo totale è grande; un guadagno opzionale rompe però la corrispondenza esatta.
- **Sincronizzazione:** gli standard Android tollerano errori di sincronizzazione dei timestamp dei sensori fino a 100 ms; va misurato sul telefono reale.
- **Rete e sicurezza:** UDP senza autenticazione su LAN; serve un token condiviso o una firma dei pacchetti, e il Wi-Fi può avere picchi di latenza. La webcam deve restare elaborata solo in locale.
- **Android:** restrizioni sui servizi in background, ottimizzazione batteria, comportamento a schermo spento; da testare su Android aggiornato.
- **Monitor:** le dimensioni esatte dell'area attiva (EDID) e le cornici influenzano la calibrazione PnP; monitor curvi sono approssimati.
- **Giochi Proton:** la compatibilità con opentrack varia da gioco a gioco; gli shortcut globali su Wayland sono limitati.
- **Interruttore di modalità:** per evitare che il cursore si muova mentre si gioca serve un controllo affidabile (scorciatoia KDE o portal GlobalShortcuts, se supportato).

## 11. Possibili miglioramenti

- **Cambio modalità automatico** in base alla finestra attiva, ad esempio tramite scripting di KWin su D-Bus (da verificare su Plasma 6.8).
- **Predizione** del movimento per ridurre la latenza apparente tra i frame della webcam.
- **Profili per gioco**, curve di risposta e zone morte configurabili.
- **Aggancio al bersaglio** (snap) sugli elementi dell'interfaccia dopo il salto del cursore, per compensare l'errore residuo.
- **Fusione più completa** (filtro di Kalman esteso) solo se i dati reali lo giustificano.
- **Alternativa senza webcam:** la camera del telefono con marker (ArUco/AprilTag) mostrati sullo schermo, per ottenere posa assoluta; più complessa.
- **Registrazione e analisi** dei dati grezzi per tarare filtri e parametri.

## 12. Domande per i revisori

1. Meglio UDP su Wi-Fi o BLE per il flusso IMU in un'applicazione di questo tipo?
2. Qualcuno ha esperienza con il puntamento assoluto via libei/portal su KDE Plasma 6?
3. Quale modo preferite per i clic con un puntatore a testa (dwell, mouse, gesto)?
4. Esperienze con la posa della testa da webcam in Rust (ONNX/`ort` o OpenCV)?
5. Il Kalman scalare per il bias dello yaw è sufficiente o conviene un filtro più completo?
6. Opinioni su fissaggio e comfort del telefono sulla testa.
7. Esperienze con opentrack e Proton: come far parlare il tracker con giochi diversi?

## 13. Riferimenti principali

- opentrack: https://github.com/opentrack/opentrack
- Documentazione portal RemoteDesktop: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.RemoteDesktop.html
- MediaPipe Face Landmarker: https://developers.google.com/edge/mediapipe/solutions/vision/face\_landmarker
- SensorEvent Android: https://developer.android.com/reference/kotlin/android/hardware/SensorEvent
- Filtro 1€ (Casiez, Roussel, Vogel, CHI 2012): https://hal.archives-ouvertes.fr/hal-00670496
- Calibrazione camera→schermo nel gaze tracking: https://arxiv.org/pdf/2009.01270
- Modifica KDE sulle coordinate assolute per stream: https://invent.kde.org/plasma/xdg-desktop-portal-kde/-/merge\_requests/210
- Crate `reis` (libei in Rust): https://docs.rs/crate/reis/latest/source/examples/receive.rs
