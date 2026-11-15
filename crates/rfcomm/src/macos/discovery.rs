use objc2_io_bluetooth::IOBluetoothDevice;

use super::Error;

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub address: String,
    pub connected: bool,
    pub paired: bool,
    pub adapter: String,
}

pub fn devices() -> Result<Vec<Device>, Error> {
    // SAFETY: pairedDevices returns an array of IOBluetoothDevice objects owned by the framework.
    let paired = unsafe { IOBluetoothDevice::pairedDevices() }.ok_or_else(|| {
        Error::Bluetooth("no paired devices; check Bluetooth permission in System Settings".into())
    })?;
    let mut found = Vec::new();
    for object in paired.iter() {
        let Some(device) = object.downcast_ref::<IOBluetoothDevice>() else {
            continue;
        };
        // SAFETY: these getters do not retain pointers beyond their returned Objective-C values.
        let name = unsafe { device.nameOrAddress() }
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".into());
        let address = unsafe { device.addressString() }
            .map(|value| value.to_string().replace('-', ":"))
            .unwrap_or_else(|| "unknown".into());
        found.push(Device {
            name,
            address,
            connected: unsafe { device.isConnected() },
            paired: unsafe { device.isPaired() },
            adapter: "IOBluetooth".into(),
        });
    }
    found.sort_by(|a, b| a.address.cmp(&b.address));
    Ok(found)
}

pub fn find_headset() -> Result<Option<Device>, Error> {
    Ok(devices()?
        .into_iter()
        .find(|device| device.name.to_uppercase().contains("MOMENTUM 4")))
}

pub fn set_headset_connected(connect: bool) -> Result<(), Error> {
    let headset =
        find_headset()?.ok_or_else(|| Error::Bluetooth("no paired MOMENTUM 4 found".to_owned()))?;
    let address = super::parse_address(&headset.address)?;
    let device = super::device(address)?;
    // SAFETY: the device is paired and remains alive for this synchronous operation.
    let status = unsafe {
        if connect {
            device.openConnection()
        } else {
            device.closeConnection()
        }
    };
    if status == 0 || status == objc2_io_bluetooth::kIOBluetoothConnectionAlreadyExists as i32 {
        Ok(())
    } else {
        Err(Error::Bluetooth(format!(
            "headset connection change failed with IOReturn {status:#010x}"
        )))
    }
}
