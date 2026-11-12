//! Emulates the phone app: streams synthetic IMU samples over UDP and
//! answers pings, so the PC-side pipeline can be tested without a phone.
//!
//! Usage: fake-phone [target=127.0.0.1:5555] [rate_hz=100]

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use hotgirl_protocol::{
    encode_pong, encode_sample, decode, Pong, Sample, HEADER_LEN, PONG_PAYLOAD_LEN,
    SAMPLE_PAYLOAD_LEN,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let target = args.get(1).cloned().unwrap_or_else(|| "127.0.0.1:5555".into());
    let rate_hz: f64 = args.get(2).map(|s| s.parse().expect("rate must be a number")).unwrap_or(100.0);

    let socket = UdpSocket::bind("0.0.0.0:0").expect("bind");
    socket.connect(&target).expect("connect");

    let ping_socket = std::sync::Arc::new(socket.try_clone().expect("clone socket"));
    std::thread::spawn(move || ping_loop(ping_socket));

    let epoch = Instant::now();
    let mut seq: u16 = 0;
    let period = Duration::from_secs_f64(1.0 / rate_hz);
    let mut buf = [0u8; HEADER_LEN + SAMPLE_PAYLOAD_LEN];
    let mut next = Instant::now();
    loop {
        let t = epoch.elapsed().as_secs_f64();
        let yaw = (t * 0.5).rem_euclid(std::f64::consts::TAU);
        let sample = Sample {
            event_ts_ns: epoch.elapsed().as_nanos() as i64,
            sent_ts_ns: epoch.elapsed().as_nanos() as i64,
            quat: [
                0.0,
                (yaw / 2.0).sin() as f32,
                0.0,
                (yaw / 2.0).cos() as f32,
            ],
            accuracy: 2.0,
        };
        encode_sample(seq, &sample, &mut buf);
        if socket.send(&buf).is_err() {
            eprintln!("send failed");
        }
        seq = seq.wrapping_add(1);
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
}

fn ping_loop(socket: std::sync::Arc<UdpSocket>) {
    let mut buf = [0u8; 64];
    let epoch = Instant::now();
    loop {
        let (len, _) = match socket.recv_from(&mut buf) {
            Ok(x) => x,
            Err(_) => continue,
        };
        if let Ok(hotgirl_protocol::Packet::Ping(ping)) = decode(&buf[..len]) {
            let pong = Pong {
                t_pc_ns: ping.t_pc_ns,
                t_phone_ns: epoch.elapsed().as_nanos() as i64,
            };
            let mut out = [0u8; HEADER_LEN + PONG_PAYLOAD_LEN];
            encode_pong(0, epoch.elapsed().as_nanos() as i64, &pong, &mut out);
            let _ = socket.send(&out);
        }
    }
}
