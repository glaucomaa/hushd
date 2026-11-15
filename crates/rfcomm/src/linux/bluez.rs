use zbus::blocking::fdo::ObjectManagerProxy;
use zbus::blocking::{Connection, Proxy};

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub address: String,
    pub connected: bool,
    pub paired: bool,
    pub adapter: String,
}

pub fn devices() -> Result<Vec<Device>, zbus::Error> {
    let connection = Connection::system()?;
    let manager = ObjectManagerProxy::new(&connection, "org.bluez", "/")?;
    let objects = manager.get_managed_objects()?;

    let mut found = Vec::new();
    for (path, interfaces) in objects {
        let Some(properties) = interfaces.get("org.bluez.Device1") else {
            continue;
        };
        let text = |key: &str| {
            properties
                .get(key)
                .and_then(|value| String::try_from(value.clone()).ok())
        };
        let flag = |key: &str| {
            properties
                .get(key)
                .and_then(|value| bool::try_from(value.clone()).ok())
        };
        let Some(address) = text("Address") else {
            continue;
        };
        let adapter = path
            .as_str()
            .split('/')
            .nth(3)
            .unwrap_or_default()
            .to_owned();
        found.push(Device {
            name: text("Alias")
                .or_else(|| text("Name"))
                .unwrap_or_else(|| address.clone()),
            address,
            connected: flag("Connected").unwrap_or(false),
            paired: flag("Paired").unwrap_or(false),
            adapter,
        });
    }
    found.sort_by(|a, b| a.address.cmp(&b.address));
    Ok(found)
}

pub fn find_headset() -> Result<Option<Device>, zbus::Error> {
    Ok(devices()?
        .into_iter()
        .find(|device| device.name.to_uppercase().contains("MOMENTUM 4")))
}

pub fn set_headset_connected(connect: bool) -> Result<(), zbus::Error> {
    let headset = find_headset()?.ok_or_else(|| {
        zbus::Error::Failure("no paired MOMENTUM 4 found on this adapter".to_owned())
    })?;
    if headset.connected == connect {
        return Ok(());
    }

    let path = format!(
        "/org/bluez/{}/dev_{}",
        headset.adapter,
        headset.address.replace(':', "_")
    );
    let connection = Connection::system()?;
    let device = Proxy::new(&connection, "org.bluez", path, "org.bluez.Device1")?;
    let method = if connect { "Connect" } else { "Disconnect" };
    let result: Result<(), zbus::Error> = device.call(method, &());
    match result {
        Err(zbus::Error::MethodError(name, _, _))
            if (connect && name.as_str() == "org.bluez.Error.AlreadyConnected")
                || (!connect && name.as_str() == "org.bluez.Error.NotConnected") =>
        {
            Ok(())
        }
        other => other,
    }
}
