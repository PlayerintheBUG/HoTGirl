//! Wire format of the phone→PC UDP stream and of the opentrack output packets.
//!
//! All integers are little-endian. Every datagram starts with a fixed-size
//! [`HEADER_LEN`] header followed by a kind-specific payload.

pub const OPENTRACK_DEFAULT_PORT: u16 = 4242;
pub const OPENTRACK_PACKET_LEN: usize = 48;

pub const HEADER_LEN: usize = 16;
pub const SAMPLE_PAYLOAD_LEN: usize = 28;
pub const PING_PAYLOAD_LEN: usize = 8;
pub const PONG_PAYLOAD_LEN: usize = 16;

pub const MAGIC: u32 = 0x4854_4752; // "HTGR"
pub const VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Sample = 1,
    Ping = 2,
    Pong = 3,
}

impl Kind {
    pub fn from_u8(v: u8) -> Option<Kind> {
        match v {
            1 => Some(Kind::Sample),
            2 => Some(Kind::Ping),
            3 => Some(Kind::Pong),
            _ => None,
        }
    }
}

/// One IMU orientation sample produced by the phone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Sensor event time, in phone `elapsedRealtimeNanos` clock.
    pub event_ts_ns: i64,
    /// Phone send time, same clock.
    pub sent_ts_ns: i64,
    /// Rotation quaternion, x/y/z/w, phone frame.
    pub quat: [f32; 4],
    /// `SensorManager.getQuaternionFromVector` style accuracy, negative if unknown.
    pub accuracy: f32,
}

/// PC→phone ping, echoed by the phone as a [`Kind::Pong`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ping {
    /// PC monotonic time at ping creation, in PC clock nanoseconds.
    pub t_pc_ns: i64,
}

/// Phone→PC reply to a [`Ping`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pong {
    /// Echoed `t_pc_ns` from the ping.
    pub t_pc_ns: i64,
    /// Phone clock at pong creation.
    pub t_phone_ns: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Packet {
    Sample(Sample),
    Ping(Ping),
    Pong(Pong),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError;

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid packet")
    }
}

impl std::error::Error for DecodeError {}

fn read_u32_le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn read_i64_le(b: &[u8], off: usize) -> i64 {
    i64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

fn read_f32_le(b: &[u8], off: usize) -> f32 {
    f32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn write_header(buf: &mut [u8], kind: Kind, seq: u16, sent_ts_ns: i64) {
    buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    buf[4] = VERSION;
    buf[5] = kind as u8;
    buf[6..8].copy_from_slice(&seq.to_le_bytes());
    buf[8..16].copy_from_slice(&sent_ts_ns.to_le_bytes());
}

pub fn encode_sample(seq: u16, s: &Sample, out: &mut [u8; HEADER_LEN + SAMPLE_PAYLOAD_LEN]) {
    write_header(out, Kind::Sample, seq, s.sent_ts_ns);
    out[16..24].copy_from_slice(&s.event_ts_ns.to_le_bytes());
    for (i, v) in s.quat.iter().enumerate() {
        out[24 + i * 4..28 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    out[40..44].copy_from_slice(&s.accuracy.to_le_bytes());
}

pub fn encode_ping(seq: u16, sent_ts_ns: i64, p: &Ping, out: &mut [u8; HEADER_LEN + PING_PAYLOAD_LEN]) {
    write_header(out, Kind::Ping, seq, sent_ts_ns);
    out[16..24].copy_from_slice(&p.t_pc_ns.to_le_bytes());
}

pub fn encode_pong(seq: u16, sent_ts_ns: i64, p: &Pong, out: &mut [u8; HEADER_LEN + PONG_PAYLOAD_LEN]) {
    write_header(out, Kind::Pong, seq, sent_ts_ns);
    out[16..24].copy_from_slice(&p.t_pc_ns.to_le_bytes());
    out[24..32].copy_from_slice(&p.t_phone_ns.to_le_bytes());
}

/// Decodes a datagram received by the PC.
pub fn decode(buf: &[u8]) -> Result<Packet, DecodeError> {
    if buf.len() < HEADER_LEN {
        return Err(DecodeError);
    }
    if read_u32_le(buf, 0) != MAGIC || buf[4] != VERSION {
        return Err(DecodeError);
    }
    let kind = Kind::from_u8(buf[5]).ok_or(DecodeError)?;
    let sent_ts_ns = read_i64_le(buf, 8);
    match kind {
        Kind::Sample => {
            if buf.len() < HEADER_LEN + SAMPLE_PAYLOAD_LEN {
                return Err(DecodeError);
            }
            Ok(Packet::Sample(Sample {
                event_ts_ns: read_i64_le(buf, 16),
                sent_ts_ns,
                quat: [
                    read_f32_le(buf, 24),
                    read_f32_le(buf, 28),
                    read_f32_le(buf, 32),
                    read_f32_le(buf, 36),
                ],
                accuracy: read_f32_le(buf, 40),
            }))
        }
        Kind::Ping => {
            if buf.len() < HEADER_LEN + PING_PAYLOAD_LEN {
                return Err(DecodeError);
            }
            Ok(Packet::Ping(Ping {
                t_pc_ns: read_i64_le(buf, 16),
            }))
        }
        Kind::Pong => {
            if buf.len() < HEADER_LEN + PONG_PAYLOAD_LEN {
                return Err(DecodeError);
            }
            Ok(Packet::Pong(Pong {
                t_pc_ns: read_i64_le(buf, 16),
                t_phone_ns: read_i64_le(buf, 24),
            }))
        }
    }
}

/// The 48-byte datagram opentrack listens for on UDP port 4242:
/// six little-endian f64, x/y/z in meters and yaw/pitch/roll in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpentrackPose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub roll: f64,
}

pub fn encode_opentrack(p: &OpentrackPose, out: &mut [u8; OPENTRACK_PACKET_LEN]) {
    let vals = [p.x, p.y, p.z, p.yaw, p.pitch, p.roll];
    for (i, v) in vals.iter().enumerate() {
        out[i * 8..(i + 1) * 8].copy_from_slice(&v.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_roundtrip() {
        let s = Sample {
            event_ts_ns: 123_456_789,
            sent_ts_ns: 124_000_000,
            quat: [0.1, -0.2, 0.3, 0.9],
            accuracy: 2.0,
        };
        let mut buf = [0u8; HEADER_LEN + SAMPLE_PAYLOAD_LEN];
        encode_sample(7, &s, &mut buf);
        match decode(&buf).unwrap() {
            Packet::Sample(got) => assert_eq!(got, s),
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn ping_pong_roundtrip() {
        let mut buf = [0u8; HEADER_LEN + PING_PAYLOAD_LEN];
        encode_ping(1, 999, &Ping { t_pc_ns: 42 }, &mut buf);
        match decode(&buf).unwrap() {
            Packet::Ping(p) => assert_eq!(p.t_pc_ns, 42),
            other => panic!("wrong kind: {other:?}"),
        }

        let mut buf = [0u8; HEADER_LEN + PONG_PAYLOAD_LEN];
        encode_pong(2, 1000, &Pong { t_pc_ns: 42, t_phone_ns: 500 }, &mut buf);
        match decode(&buf).unwrap() {
            Packet::Pong(p) => {
                assert_eq!(p.t_pc_ns, 42);
                assert_eq!(p.t_phone_ns, 500);
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(&[]).is_err());
        assert!(decode(&[0u8; 4]).is_err());
        let mut buf = [0u8; HEADER_LEN + SAMPLE_PAYLOAD_LEN];
        encode_sample(0, &Sample {
            event_ts_ns: 0,
            sent_ts_ns: 0,
            quat: [0.0; 4],
            accuracy: -1.0,
        }, &mut buf);
        buf[4] = 9;
        assert!(decode(&buf).is_err());
    }

    #[test]
    fn opentrack_layout() {
        let p = OpentrackPose { x: 0.0, y: 0.0, z: 0.0, yaw: 12.5, pitch: -3.25, roll: 1.0 };
        let mut buf = [0u8; OPENTRACK_PACKET_LEN];
        encode_opentrack(&p, &mut buf);
        assert_eq!(buf.len(), 48);
        let yaw = f64::from_le_bytes(buf[24..32].try_into().unwrap());
        assert_eq!(yaw, 12.5);
    }
}
