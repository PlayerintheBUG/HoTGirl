//! Spike: absolute cursor positioning on Wayland/KDE Plasma via the
//! RemoteDesktop portal (`ConnectToEIS`) and libei, using `reis`.
//!
//! After the portal dialogs, it sweeps the cursor over the four corners of
//! every monitor region once per second. Run it on a real Plasma session:
//! this is the highest-risk part of the plan and must be verified there.

use std::os::unix::net::UnixStream;
use std::time::Duration;

use ashpd::desktop::remote_desktop::{
    ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode};
use enumflags2::BitFlags;
use futures_util::StreamExt;
use reis::ei;
use reis::ei::device::Device as EiDevice;
use reis::ei::pointer_absolute::PointerAbsolute;
use reis::event::{DeviceCapability, EiEvent};

async fn open_ei_context() -> ei::Context {
    if let Some(context) = ei::Context::connect_to_env().expect("ei socket") {
        eprintln!("Using EI socket from environment");
        return context;
    }

    eprintln!("No EI socket in environment, going through the RemoteDesktop portal");
    let remote_desktop = RemoteDesktop::new().await.expect("portal");
    let session = remote_desktop
        .create_session(CreateSessionOptions::default())
        .await
        .expect("create session");
    let options = SelectDevicesOptions::default()
        .set_devices(BitFlags::from(DeviceType::Pointer))
        .set_persist_mode(PersistMode::DoNot);
    remote_desktop
        .select_devices(&session, options)
        .await
        .expect("select devices");
    remote_desktop
        .start(&session, None, StartOptions::default())
        .await
        .expect("start session");
    let fd = remote_desktop
        .connect_to_eis(&session, ConnectToEISOptions::default())
        .await
        .expect("connect_to_eis");
    ei::Context::new(UnixStream::from(fd)).expect("ei context")
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let context = open_ei_context().await;
    let (connection, mut events) = context
        .handshake_tokio("spike-cursor", ei::handshake::ContextType::Sender)
        .await
        .expect("handshake");

    eprintln!("Waiting for device (approve the portal dialogs)...");

    let mut pointer: Option<PointerAbsolute> = None;
    let mut ei_device: Option<EiDevice> = None;
    let mut regions: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut last_serial: u32;
    let mut sequence: u32 = 0;
    let mut sweeping = false;

    while let Some(event) = events.next().await {
        match event.expect("ei event") {
            EiEvent::SeatAdded(seat_added) => {
                seat_added
                    .seat
                    .bind_capabilities(BitFlags::from(DeviceCapability::PointerAbsolute));
                let _ = connection.flush();
            }
            EiEvent::DeviceAdded(device_added) => {
                let dev = &device_added.device;
                eprintln!(
                    "Device: {:?}, type {:?}, regions {:?}",
                    dev.name(),
                    dev.device_type(),
                    dev.regions()
                );
                pointer = dev.interface::<PointerAbsolute>();
                ei_device = Some(dev.device().clone());
                regions = dev
                    .regions()
                    .iter()
                    .map(|r| (r.x as f32, r.y as f32, r.width as f32, r.height as f32))
                    .collect();
            }
            EiEvent::DeviceResumed(resumed) => {
                last_serial = resumed.serial;
                if sweeping {
                    continue;
                }
                let (Some(pointer), Some(device)) = (pointer.clone(), ei_device.clone()) else {
                    continue;
                };
                sweeping = true;
                sequence = sequence.wrapping_add(1);
                let seq = sequence;
                let serial = last_serial;
                let sweep_regions = regions.clone();
                let sweep_conn = connection.clone();
                tokio::spawn(async move {
                    corner_sweep(sweep_conn, pointer, device, sweep_regions, seq, serial).await;
                });
            }
            EiEvent::Disconnected(disconnected) => {
                eprintln!("Disconnected: {:?}", disconnected.reason);
                break;
            }
            _ => {}
        }
    }
}

async fn corner_sweep(
    connection: reis::event::Connection,
    pointer: PointerAbsolute,
    device: EiDevice,
    regions: Vec<(f32, f32, f32, f32)>,
    sequence: u32,
    serial: u32,
) {
    let corners: Vec<(f32, f32)> = regions
        .iter()
        .flat_map(|&(x, y, w, h)| {
            [
                (x, y),
                (x + w - 1.0, y),
                (x + w - 1.0, y + h - 1.0),
                (x, y + h - 1.0),
            ]
        })
        .collect();

    device.start_emulating(serial, sequence);
    if let Err(e) = connection.flush() {
        eprintln!("flush after start_emulating failed: {e}");
    }
    loop {
        for (x, y) in &corners {
            pointer.motion_absolute(*x, *y);
            device.frame(serial, 0);
            if let Err(e) = connection.flush() {
                eprintln!("flush failed: {e}");
            }
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
    }
}
