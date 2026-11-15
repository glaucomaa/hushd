use std::sync::Arc;
use std::time::{Duration, Instant};

use objc2::runtime::ProtocolObject;
use objc2_io_bluetooth::IOBluetoothSDPUUID;

use super::{bluetooth_error, device, run_loop_slice, ChannelDelegate, Error, Shared};

pub fn resolve(address: [u8; 6], service_uuid: [u8; 16]) -> Result<Vec<u8>, Error> {
    let device = device(address)?;
    let shared = Arc::new(Shared::default());
    let delegate = ChannelDelegate::new(Arc::clone(&shared));
    // SAFETY: the delegate remains alive on this thread until the asynchronous query completes.
    let status = unsafe {
        device.performSDPQuery(Some(
            ProtocolObject::<dyn objc2_io_bluetooth::IOBluetoothDeviceAsyncCallbacks>::from_ref(
                &*delegate,
            )
            .as_ref(),
        ))
    };
    if status != 0 {
        return Err(bluetooth_error("start SDP query", status));
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = shared
            .state
            .lock()
            .map_err(|_| Error::Bluetooth("SDP state is poisoned".into()))?
            .sdp_status
        {
            if status != 0 {
                return Err(bluetooth_error("SDP query", status));
            }
            break;
        }
        if Instant::now() >= deadline {
            return Err(Error::Bluetooth("SDP query timed out".into()));
        }
        run_loop_slice();
    }
    // SAFETY: the UUID points to exactly 16 readable bytes for this call.
    let uuid = unsafe {
        IOBluetoothSDPUUID::uuidWithBytes_length(
            service_uuid.as_ptr().cast(),
            service_uuid.len() as u32,
        )
    }
    .ok_or_else(|| Error::Bluetooth("could not create the GAIA SDP UUID".into()))?;
    // SAFETY: the completed SDP cache belongs to this device and is queried synchronously.
    let record = unsafe { device.getServiceRecordForUUID(Some(&uuid)) }.ok_or_else(|| {
        Error::Bluetooth("the GAIA UUID was not present in the SDP response".into())
    })?;
    let mut channel = 0;
    // SAFETY: `channel` is a valid out-pointer for a BluetoothRFCOMMChannelID.
    let status = unsafe { record.getRFCOMMChannelID(&mut channel) };
    if status != 0 {
        return Err(bluetooth_error("read RFCOMM channel from SDP", status));
    }
    if channel == 0 {
        return Err(Error::Bluetooth("SDP returned RFCOMM channel 0".into()));
    }
    let mut channels = vec![channel];
    for fallback in [1, 15] {
        if !channels.contains(&fallback) {
            channels.push(fallback);
        }
    }
    Ok(channels)
}
