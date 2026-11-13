//! Reads a CSV recorded by hotgirl-app (recv_ns,event_ns,seq,qx,qy,qz,qw)
//! and prints measurement statistics: rate, packet loss, jitter, transport
//! delay and yaw drift.
//!
//! Usage: analyze-log <file.csv>

use std::io::{BufRead, BufReader};

use hotgirl_core::quat_to_pose;

struct Row {
    recv_ns: i64,
    event_ns: i64,
    seq: u16,
    quat: [f32; 4],
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: analyze-log <file.csv>");
    let file = std::fs::File::open(&path).expect("open csv");
    let rows: Vec<Row> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let parts: Vec<&str> = l.split(',').collect();
            if parts.len() != 8 {
                return None;
            }
            let n = |i: usize| -> Option<f64> { parts[i].trim().parse::<f64>().ok() };
            Some(Row {
                recv_ns: n(0)? as i64,
                event_ns: n(1)? as i64,
                seq: n(2)? as u16,
                quat: [n(3)? as f32, n(4)? as f32, n(5)? as f32, n(6)? as f32],
            })
        })
        .collect();

    if rows.len() < 10 {
        eprintln!("Troppi pochi campioni ({})", rows.len());
        return;
    }

    println!("Campioni: {}", rows.len());

    let duration_s = (rows.last().unwrap().recv_ns - rows.first().unwrap().recv_ns) as f64 / 1e9;
    println!("Durata: {duration_s:.1} s");
    println!("Frequenza media: {:.1} Hz", rows.len() as f64 / duration_s);

    let mut lost = 0u64;
    for pair in rows.windows(2) {
        let gap = pair[1].seq.wrapping_sub(pair[0].seq);
        if gap > 1 && gap < 60_000 {
            lost += (gap - 1) as u64;
        }
    }
    let total = rows.len() as f64 + lost as f64;
    println!(
        "Pacchetti persi: {lost} ({:.2}%)",
        100.0 * lost as f64 / total
    );

    let mut deltas: Vec<f64> = rows
        .windows(2)
        .map(|p| (p[1].recv_ns - p[0].recv_ns) as f64 / 1e6)
        .collect();
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean_ms = deltas.iter().sum::<f64>() / deltas.len() as f64;
    let var = deltas.iter().map(|d| (d - mean_ms).powi(2)).sum::<f64>() / deltas.len() as f64;
    let p95 = deltas[(deltas.len() as f64 * 0.95) as usize % deltas.len()];
    println!(
        "Inter-arrivo: medio {:.2} ms, jitter (std) {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        mean_ms,
        var.sqrt(),
        p95,
        *deltas.last().unwrap()
    );

    let delays: Vec<f64> = rows
        .iter()
        .map(|r| (r.recv_ns - r.event_ns) as f64 / 1e6)
        .collect();
    let dmean = delays.iter().sum::<f64>() / delays.len() as f64;
    let dvar = delays.iter().map(|d| (d - dmean).powi(2)).sum::<f64>() / delays.len() as f64;
    println!("Ritardo trasporto (recv−event): medio {:.2} ms, std {:.2} ms", dmean, dvar.sqrt());

    let yaws: Vec<f64> = rows
        .iter()
        .map(|r| quat_to_pose(r.quat).yaw_deg)
        .collect();
    let mut unwrapped = vec![yaws[0]];
    for &y in &yaws[1..] {
        let mut prev = *unwrapped.last().unwrap();
        while y - prev > 180.0 {
            prev += 360.0;
        }
        while y - prev < -180.0 {
            prev -= 360.0;
        }
        unwrapped.push(y);
    }
    let n = unwrapped.len() as f64;
    let xs: Vec<f64> = (0..unwrapped.len()).map(|i| i as f64).collect();
    let xm = xs.iter().sum::<f64>() / n;
    let ym = unwrapped.iter().sum::<f64>() / n;
    let cov: f64 = xs.iter().zip(&unwrapped).map(|(x, y)| (x - xm) * (y - ym)).sum();
    let varx: f64 = xs.iter().map(|x| (x - xm).powi(2)).sum();
    let slope_per_sample = cov / varx;
    let slope_deg_per_min = slope_per_sample * 60.0 * (rows.len() as f64 / duration_s);
    println!("Deriva yaw: {:.2} °/min (fit lineare)", slope_deg_per_min);

    let detrended: Vec<f64> = unwrapped
        .iter()
        .zip(&xs)
        .map(|(y, x)| y - slope_per_sample * (x - xm) - ym)
        .collect();
    let dvar2 = detrended.iter().map(|d| d.powi(2)).sum::<f64>() / n;
    println!("Yaw detrended: std {:.3}°, max |Δ| {:.3}°", dvar2.sqrt(), detrended.iter().fold(0.0f64, |m, d| m.max(d.abs())));
}
