use std::io::BufWriter;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use hotgirl_core::{ClockSync, Pose, SampleBuffer, quat_to_pose};
use hotgirl_protocol::{
    OpentrackPose, Packet, Ping, PING_PAYLOAD_LEN, OPENTRACK_PACKET_LEN,
};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let listen_port: u16 = args
        .get(1)
        .map(|s| s.parse().expect("listen port must be a number"))
        .unwrap_or(5555);
    let record_file = args.get(2).map(std::path::PathBuf::from);

    let epoch = Instant::now();
    let state = Arc::new(AppState::new());
    let socket =
        Arc::new(UdpSocket::bind(("0.0.0.0", listen_port)).expect("bind UDP listen socket"));

    let recorder = Arc::new(Mutex::new(
        record_file
            .map(|path| {
                std::fs::File::create(&path)
                    .map(BufWriter::new)
                    .expect("create record file")
            }),
    ));

    let rx_state = Arc::clone(&state);
    let rx_socket = Arc::clone(&socket);
    let rx_recorder = Arc::clone(&recorder);
    thread::spawn(move || receive_loop(rx_socket, rx_state, epoch, rx_recorder));

    let tx_state = Arc::clone(&state);
    let tx_socket = Arc::clone(&socket);
    thread::spawn(move || ping_loop(tx_socket, tx_state, epoch));

    let ot_state = Arc::clone(&state);
    thread::spawn(move || opentrack_loop(ot_state));

    let ui_state = Arc::clone(&state);
    let mut options = eframe::NativeOptions::default();
    options.viewport.inner_size = Some((520.0, 360.0).into());
    eframe::run_native(
        "HoTGirl",
        options,
        Box::new(move |_cc| Ok(Box::new(App { state: ui_state }))),
    )
}

pub struct AppState {
    pub clock_sync: Mutex<ClockSync>,
    pub buffer: Mutex<SampleBuffer>,
    pub phone_addr: Mutex<Option<std::net::SocketAddr>>,
    pub pose: Mutex<Pose>,
    pub stats: Stats,
    pub opentrack_enabled: AtomicBool,
    pub opentrack_addr: Mutex<String>,
    pub recorder: Mutex<Option<BufWriter<std::fs::File>>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            clock_sync: Mutex::new(ClockSync::new()),
            buffer: Mutex::new(SampleBuffer::new(4096)),
            phone_addr: Mutex::new(None),
            pose: Mutex::new(Pose::default()),
            stats: Stats::default(),
            opentrack_enabled: AtomicBool::new(false),
            opentrack_addr: Mutex::new("127.0.0.1:4242".to_string()),
            recorder: Mutex::new(None),
        }
    }
}

#[derive(Default)]
pub struct Stats {
    pub valid_packets: AtomicU64,
    pub invalid_packets: AtomicU64,
    pub samples: AtomicU64,
    pub last_recv_ms: AtomicU64,
}

struct App {
    state: Arc<AppState>,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let state = &self.state;

            ui.heading("HoTGirl – head tracker");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Telefono:");
                match *state.phone_addr.lock().unwrap() {
                    Some(addr) => ui.label(addr.to_string()),
                    None => ui.weak("non connesso"),
                };
            });

            let pose = *state.pose.lock().unwrap();
            egui::Grid::new("pose").num_columns(2).show(ui, |ui| {
                ui.label("Yaw");
                ui.label(format!("{:+7.2}°", pose.yaw_deg));
                ui.end_row();
                ui.label("Pitch");
                ui.label(format!("{:+7.2}°", pose.pitch_deg));
                ui.end_row();
                ui.label("Roll");
                ui.label(format!("{:+7.2}°", pose.roll_deg));
                ui.end_row();
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Pacchetti validi:");
                ui.label(state.stats.valid_packets.load(Ordering::Relaxed).to_string());
                ui.label("non validi:");
                ui.label(state.stats.invalid_packets.load(Ordering::Relaxed).to_string());
                ui.label("campioni:");
                ui.label(state.stats.samples.load(Ordering::Relaxed).to_string());
            });

            if let Some(offset) = state.clock_sync.lock().unwrap().offset_ns() {
                ui.label(format!("Offset orologi (telefono−PC): {offset} ns"));
            } else {
                ui.weak("Offset orologi: in attesa di pong");
            }

            ui.separator();
            ui.horizontal(|ui| {
                let mut enabled = state.opentrack_enabled.load(Ordering::Relaxed);
                if ui.checkbox(&mut enabled, "Invia a opentrack").changed() {
                    state.opentrack_enabled.store(enabled, Ordering::Relaxed);
                }
                let mut addr = state.opentrack_addr.lock().unwrap().clone();
                if ui.text_edit_singleline(&mut addr).changed() {
                    *state.opentrack_addr.lock().unwrap() = addr;
                }
            });

            if let Some(sample) = state.buffer.lock().unwrap().latest() {
                ui.label(format!(
                    "Ultimo campione: event={:+.3} ms  quat=({:+.3}, {:+.3}, {:+.3}, {:+.3})",
                    sample.event_ts_ns as f64 / 1e6,
                    sample.quat[0],
                    sample.quat[1],
                    sample.quat[2],
                    sample.quat[3],
                ));
            }

            ctx.request_repaint_after(Duration::from_millis(33));
        });
    }
}

fn pc_now_ns(epoch: &Instant) -> i64 {
    epoch.elapsed().as_nanos() as i64
}

fn receive_loop(
    socket: Arc<UdpSocket>,
    state: Arc<AppState>,
    epoch: Instant,
    recorder: Arc<Mutex<Option<BufWriter<std::fs::File>>>>,
) {
    let mut buf = [0u8; 1500];
    loop {
        let (len, addr) = match socket.recv_from(&mut buf) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let packet = match hotgirl_protocol::decode(&buf[..len]) {
            Ok(p) => p,
            Err(_) => {
                state.stats.invalid_packets.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };
        state.stats.valid_packets.fetch_add(1, Ordering::Relaxed);
        state.stats.last_recv_ms.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
        *state.phone_addr.lock().unwrap() = Some(addr);

        match packet {
            Packet::Sample(sample) => {
                state.stats.samples.fetch_add(1, Ordering::Relaxed);
                let offset = state.clock_sync.lock().unwrap().offset_ns().unwrap_or(0);
                let event_pc = sample.event_ts_ns - offset;
                state.buffer.lock().unwrap().push(event_pc, sample);
                *state.pose.lock().unwrap() = quat_to_pose(sample.quat);
                if let Some(writer) = recorder.lock().unwrap().as_mut() {
                    use std::io::Write;
                    let recv_ns = pc_now_ns(&epoch);
                    let q = sample.quat;
                    let _ = writeln!(
                        writer,
                        "{recv_ns},{event_pc},{},{},{},{},{},{}",
                        sample.seq,
                        q[0],
                        q[1],
                        q[2],
                        q[3],
                        sample.accuracy
                    );
                }
            }
            Packet::Pong(pong) => {
                let now = pc_now_ns(&epoch);
                state.clock_sync.lock().unwrap().on_pong(&pong, now);
            }
            Packet::Ping(_) => {}
        }
    }
}

fn ping_loop(socket: Arc<UdpSocket>, state: Arc<AppState>, epoch: Instant) {
    let mut seq: u16 = 0;
    loop {
        if let Some(addr) = *state.phone_addr.lock().unwrap() {
            let t_pc = pc_now_ns(&epoch);
            let mut buf = [0u8; hotgirl_protocol::HEADER_LEN + PING_PAYLOAD_LEN];
            hotgirl_protocol::encode_ping(seq, t_pc, &Ping { t_pc_ns: t_pc }, &mut buf);
            let _ = socket.send_to(&buf, addr);
            seq = seq.wrapping_add(1);
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn opentrack_loop(state: Arc<AppState>) {
    loop {
        thread::sleep(Duration::from_millis(5));
        if !state.opentrack_enabled.load(Ordering::Relaxed) {
            continue;
        }
        let pose = *state.pose.lock().unwrap();
        let ot = OpentrackPose {
            x: pose.x,
            y: pose.y,
            z: pose.z,
            yaw: pose.yaw_deg,
            pitch: pose.pitch_deg,
            roll: pose.roll_deg,
        };
        let mut buf = [0u8; OPENTRACK_PACKET_LEN];
        hotgirl_protocol::encode_opentrack(&ot, &mut buf);
        let addr = state.opentrack_addr.lock().unwrap().clone();
        if let Ok(target) = addr.parse::<std::net::SocketAddr>() {
            let _ = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| s.send_to(&buf, target));
        }
    }
}
