use std::ffi::{c_int, c_void};
use std::io;
use std::ptr;

const SDP_RETRY_IF_BUSY: u32 = 0x01;
const SDP_ATTR_REQ_RANGE: c_int = 2;
const RFCOMM_UUID: c_int = 0x0003;

#[repr(C, packed)]
struct BdAddr {
    bytes: [u8; 6],
}

#[repr(C)]
union UuidValue {
    uuid16: u16,
    uuid32: u32,
    uuid128: [u8; 16],
}

#[repr(C)]
struct Uuid {
    kind: u8,
    value: UuidValue,
}

#[repr(C)]
struct SdpList {
    next: *mut SdpList,
    data: *mut c_void,
}

#[repr(C)]
struct SdpSession {
    _private: [u8; 0],
}

#[repr(C)]
struct SdpRecord {
    _private: [u8; 0],
}

#[link(name = "bluetooth")]
extern "C" {
    fn sdp_connect(src: *const BdAddr, dst: *const BdAddr, flags: u32) -> *mut SdpSession;
    fn sdp_close(session: *mut SdpSession) -> c_int;
    fn sdp_uuid128_create(uuid: *mut Uuid, data: *const c_void) -> *mut Uuid;
    fn sdp_list_append(list: *mut SdpList, data: *mut c_void) -> *mut SdpList;
    fn sdp_list_free(list: *mut SdpList, free: Option<unsafe extern "C" fn(*mut c_void)>);
    fn sdp_service_search_attr_req(
        session: *mut SdpSession,
        search: *const SdpList,
        request_type: c_int,
        attributes: *const SdpList,
        response: *mut *mut SdpList,
    ) -> c_int;
    fn sdp_record_free(record: *mut SdpRecord);
    fn sdp_get_access_protos(record: *const SdpRecord, protocols: *mut *mut SdpList) -> c_int;
    fn sdp_get_proto_port(list: *const SdpList, protocol: c_int) -> c_int;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SDP connection failed: {0}")]
    Connect(#[source] io::Error),
    #[error("SDP service search failed: {0}")]
    Search(#[source] io::Error),
    #[error("no RFCOMM channels found for the requested SDP service")]
    NoChannels,
}

struct Session(*mut SdpSession);

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the pointer comes from a successful sdp_connect and is closed once.
        unsafe {
            sdp_close(self.0);
        }
    }
}

struct Records(*mut SdpList);

unsafe extern "C" fn free_record(record: *mut c_void) {
    sdp_record_free(record.cast::<SdpRecord>());
}

impl Drop for Records {
    fn drop(&mut self) {
        // SAFETY: this list and its records belong to the completed SDP search.
        unsafe {
            sdp_list_free(self.0, Some(free_record));
        }
    }
}

pub fn resolve(address: [u8; 6], service_uuid: [u8; 16]) -> Result<Vec<u8>, Error> {
    let mut target = BdAddr { bytes: address };
    target.bytes.reverse();
    let any = BdAddr { bytes: [0; 6] };

    // SAFETY: all pointers remain valid for their respective BlueZ calls. The
    // search and attribute lists are freed immediately after the request; the
    // returned record list and session are owned by RAII guards.
    unsafe {
        let raw_session = sdp_connect(&any, &target, SDP_RETRY_IF_BUSY);
        if raw_session.is_null() {
            return Err(Error::Connect(io::Error::last_os_error()));
        }
        let _session = Session(raw_session);

        let mut uuid: Uuid = std::mem::zeroed();
        sdp_uuid128_create(&mut uuid, service_uuid.as_ptr().cast::<c_void>());
        let mut all_attributes = 0x0000_ffff_u32;
        let search = sdp_list_append(ptr::null_mut(), (&mut uuid as *mut Uuid).cast::<c_void>());
        let attributes = sdp_list_append(
            ptr::null_mut(),
            (&mut all_attributes as *mut u32).cast::<c_void>(),
        );
        let mut raw_records = ptr::null_mut();
        let result = sdp_service_search_attr_req(
            raw_session,
            search,
            SDP_ATTR_REQ_RANGE,
            attributes,
            &mut raw_records,
        );
        sdp_list_free(search, None);
        sdp_list_free(attributes, None);
        if result != 0 {
            return Err(Error::Search(io::Error::last_os_error()));
        }
        let records = Records(raw_records);

        let mut channels = Vec::new();
        let mut node = records.0;
        while let Some(entry) = node.as_ref() {
            let record = entry.data.cast::<SdpRecord>();
            let mut protocols = ptr::null_mut();
            if !record.is_null() && sdp_get_access_protos(record, &mut protocols) == 0 {
                let channel = sdp_get_proto_port(protocols, RFCOMM_UUID);
                sdp_list_free(protocols, None);
                if (1..=30).contains(&channel) && !channels.contains(&(channel as u8)) {
                    channels.push(channel as u8);
                }
            }
            node = entry.next;
        }
        if channels.is_empty() {
            return Err(Error::NoChannels);
        }
        Ok(channels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluez_value_layouts_match_the_c_headers() {
        assert_eq!(std::mem::size_of::<BdAddr>(), 6);
        assert_eq!(std::mem::size_of::<Uuid>(), 20);
        assert_eq!(std::mem::align_of::<Uuid>(), 4);
    }
}
