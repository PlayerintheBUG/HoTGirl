//! Core state for the head tracker: clock synchronization, sample buffering
//! and pose extraction. No I/O here; transport and UI live elsewhere.

use std::collections::VecDeque;

use nalgebra::{Matrix3, Quaternion, UnitQuaternion};

use hotgirl_protocol::{Pong, Sample};

/// Estimates the phone-clock offset from ping/pong exchanges.
///
/// For each pong: `rtt = recv_ts - t_pc` (PC clock) and
/// `offset = t_phone - (t_pc + recv_ts) / 2`, i.e. phone clock minus PC clock,
/// assuming the phone answers instantly. Only pongs with the smallest round
/// trips in a sliding window are trusted, because those are the ones least
/// distorted by queueing.
#[derive(Debug, Clone)]
pub struct ClockSync {
    window: VecDeque<(u64, i64)>,
    capacity: usize,
}

impl ClockSync {
    pub fn new() -> Self {
        Self::with_capacity(64)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            window: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// Feeds a pong and returns `(rtt_ns, offset_ns)`.
    pub fn on_pong(&mut self, pong: &Pong, recv_ts_ns: i64) -> (u64, i64) {
        let rtt = (recv_ts_ns - pong.t_pc_ns).max(0) as u64;
        let offset = pong.t_phone_ns - (pong.t_pc_ns + recv_ts_ns) / 2;
        self.window.push_back((rtt, offset));
        while self.window.len() > self.capacity {
            self.window.pop_front();
        }
        (rtt, offset)
    }

    /// Phone clock minus PC clock, in nanoseconds, from the lowest-RTT pongs.
    /// Returns `None` until at least one pong has been received.
    pub fn offset_ns(&self) -> Option<i64> {
        if self.window.is_empty() {
            return None;
        }
        let best_rtt = self.window.iter().map(|&(rtt, _)| rtt).min().unwrap();
        // Tolerate samples within 1 ms of the best round trip.
        let tol = best_rtt.saturating_add(1_000_000);
        let mut sum = 0i128;
        let mut n = 0u32;
        for &(rtt, offset) in &self.window {
            if rtt <= tol {
                sum += offset as i128;
                n += 1;
            }
        }
        Some((sum / n as i128) as i64)
    }

    pub fn best_rtt_ns(&self) -> Option<u64> {
        self.window.iter().map(|&(rtt, _)| rtt).min()
    }
}

impl Default for ClockSync {
    fn default() -> Self {
        Self::new()
    }
}

/// Time-ordered ring buffer of IMU samples, keyed by event time in PC clock.
#[derive(Debug)]
pub struct SampleBuffer {
    samples: VecDeque<(i64, Sample)>,
    capacity: usize,
}

impl SampleBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    /// Inserts a sample whose event time has been converted to the PC clock.
    pub fn push(&mut self, event_ts_pc_ns: i64, sample: Sample) {
        while let Some(front) = self.samples.front() {
            if front.0 > event_ts_pc_ns {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        self.samples.push_back((event_ts_pc_ns, sample));
        while self.samples.len() > self.capacity {
            self.samples.pop_front();
        }
    }

    pub fn latest(&self) -> Option<&Sample> {
        self.samples.back().map(|(_, s)| s)
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// The newest sample with `event_ts_pc_ns <= ts`, for aligning slower
    /// webcam frames with the IMU stream.
    pub fn at_or_before(&self, ts: i64) -> Option<&Sample> {
        self.samples
            .iter()
            .rev()
            .find(|&&(t, _)| t <= ts)
            .map(|(_, s)| s)
    }
}

/// Head pose in opentrack convention: x/y/z in meters, yaw/pitch/roll in
/// degrees. Yaw rotates about the vertical (Y up) axis, pitch about the
/// horizontal (X right) axis, roll about the view axis (Z).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
}

/// Decomposes a rotation into yaw/pitch/roll degrees, composed as
/// `R = Ry(yaw) * Rx(pitch) * Rz(roll)` in a Y-up, Z-toward-user world frame.
pub fn quat_to_pose(quat: [f32; 4]) -> Pose {
    let q = UnitQuaternion::from_quaternion(Quaternion::new(
        quat[3] as f64,
        quat[0] as f64,
        quat[1] as f64,
        quat[2] as f64,
    ));
    let m: Matrix3<f64> = q.to_rotation_matrix().into();
    let m01 = m[(0, 2)];
    let m11 = m[(1, 1)];
    let m21 = m[(2, 2)];
    let m10 = m[(1, 0)];
    let m12 = m[(1, 2)];
    let pitch = (-m12).clamp(-1.0, 1.0).asin();
    let yaw = m01.atan2(m21);
    let roll = m10.atan2(m11);
    Pose {
        yaw_deg: yaw.to_degrees(),
        pitch_deg: pitch.to_degrees(),
        roll_deg: roll.to_degrees(),
        ..Pose::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::{Rotation3, UnitQuaternion};

    fn quat_from_ypr(yaw_deg: f64, pitch_deg: f64, roll_deg: f64) -> [f32; 4] {
        let rot = Rotation3::new((yaw_deg.to_radians()) * nalgebra::Vector3::y())
            * Rotation3::new((pitch_deg.to_radians()) * nalgebra::Vector3::x())
            * Rotation3::new((roll_deg.to_radians()) * nalgebra::Vector3::z());
        let q = UnitQuaternion::from_rotation_matrix(&rot);
        [q.i as f32, q.j as f32, q.k as f32, q.w as f32]
    }

    #[test]
    fn clock_sync_offset_and_rtt() {
        let mut sync = ClockSync::new();
        // PC clock at ping send = 1_000_000; phone answers at phone 5_000_000.
        // PC receives at 1_400_000 → rtt 400_000,
        // offset = 5_000_000 - 1_200_000 = 3_800_000.
        let pong = Pong { t_pc_ns: 1_000_000, t_phone_ns: 5_000_000 };
        let (rtt, offset) = sync.on_pong(&pong, 1_400_000);
        assert_eq!(rtt, 400_000);
        assert_eq!(offset, 3_800_000);
        assert_eq!(sync.offset_ns(), Some(3_800_000));
        assert_eq!(sync.best_rtt_ns(), Some(400_000));
    }

    #[test]
    fn clock_sync_prefers_low_rtt() {
        let mut sync = ClockSync::with_capacity(4);
        sync.on_pong(&Pong { t_pc_ns: 0, t_phone_ns: 10_000_000 }, 10_000_000);
        sync.on_pong(&Pong { t_pc_ns: 20_000_000, t_phone_ns: 35_000_000 }, 20_100_000);
        // Second pong: rtt 100_000, offset 14_950_000, wins over the noisy first.
        assert_eq!(sync.offset_ns(), Some(14_950_000));
    }

    #[test]
    fn sample_buffer_ordered_access() {
        let mut buf = SampleBuffer::new(8);
        let mk = |ts: i64| Sample {
            event_ts_ns: ts,
            sent_ts_ns: ts,
            quat: [0.0, 0.0, 0.0, 1.0],
            accuracy: -1.0,
        };
        buf.push(100, mk(100));
        buf.push(200, mk(200));
        buf.push(300, mk(300));
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.at_or_before(250).unwrap().event_ts_ns, 200);
        assert_eq!(buf.at_or_before(50), None);
        assert_eq!(buf.latest().unwrap().event_ts_ns, 300);
    }

    #[test]
    fn pose_roundtrip() {
        for &(y, p, r) in &[
            (0.0, 0.0, 0.0),
            (30.0, 0.0, 0.0),
            (0.0, 15.0, 0.0),
            (0.0, 0.0, -10.0),
            (25.0, 12.0, 5.0),
            (-40.0, -20.0, 30.0),
        ] {
            let pose = quat_to_pose(quat_from_ypr(y, p, r));
            assert!((pose.yaw_deg - y).abs() < 1e-4, "yaw {y} vs {}", pose.yaw_deg);
            assert!((pose.pitch_deg - p).abs() < 1e-4, "pitch {p} vs {}", pose.pitch_deg);
            assert!((pose.roll_deg - r).abs() < 1e-4, "roll {r} vs {}", pose.roll_deg);
        }
    }
}
