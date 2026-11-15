pub const SENNHEISER: u16 = 0x0495;
pub const QUALCOMM: u16 = 0x001d;

pub mod feature {
    pub const CORE: u8 = 0;
    pub const UPGRADE: u8 = 1;
    pub const DEVICE: u8 = 2;
    pub const BATTERY: u8 = 3;
    pub const AUDIO: u8 = 4;
    pub const EQ: u8 = 8;
    pub const VERSIONS: u8 = 9;
    pub const CONNECTIONS: u8 = 10;
    pub const MMI: u8 = 11;
    pub const TRANSPARENCY: u8 = 12;
    pub const ANC: u8 = 13;
    pub const PERSONALIZATION: u8 = 16;
}

pub mod cmd {
    pub const REGISTER_NOTIFICATION: u16 = 0x0007;
    pub const FACTORY_RESET: u16 = 0x0040;

    pub const ON_HEAD_SET: u16 = 0x0400;
    pub const ON_HEAD_GET: u16 = 0x0401;
    pub const PHYSICAL_STATE_GET: u16 = 0x0402;
    pub const BT_MODE_SET: u16 = 0x0405;
    pub const BT_MODE_GET: u16 = 0x0406;

    pub const TIMER_SET: u16 = 0x0600;
    pub const TIMER_GET: u16 = 0x0601;
    pub const CHARGING_GET: u16 = 0x0602;
    pub const BATTERY_GET: u16 = 0x0603;

    pub const CODEC_GET: u16 = 0x0800;
    pub const AUDIO_MODE_SET: u16 = 0x0803;
    pub const AUDIO_MODE_GET: u16 = 0x0804;
    pub const PROMPT_MODE_SET: u16 = 0x0801;
    pub const PROMPT_MODE_GET: u16 = 0x0802;
    pub const SIDETONE_SET: u16 = 0x0805;
    pub const SIDETONE_GET: u16 = 0x0806;
    pub const PROMPT_LANGUAGE_GET: u16 = 0x0807;
    pub const AUTO_CALL_SET: u16 = 0x080a;
    pub const AUTO_CALL_GET: u16 = 0x080b;
    pub const SMART_PAUSE_SET: u16 = 0x080c;
    pub const SMART_PAUSE_GET: u16 = 0x080d;
    pub const COMFORT_CALL_SET: u16 = 0x0814;
    pub const COMFORT_CALL_GET: u16 = 0x0815;
    pub const LOW_LATENCY_SET: u16 = 0x0817;
    pub const LOW_LATENCY_GET: u16 = 0x0818;
    pub const SAMPLE_RATE_GET: u16 = 0x081a;

    pub const EQ_CONFIG_GET: u16 = 0x1000;
    pub const EQ_SLIDER_SET: u16 = 0x1001;
    pub const EQ_SLIDER_GET: u16 = 0x1002;
    pub const EQ_SUBMODE_SET: u16 = 0x1008;
    pub const EQ_SUBMODE_GET: u16 = 0x1009;

    pub const HW_REVISION_GET: u16 = 0x1200;
    pub const FW_VERSION_GET: u16 = 0x1201;
    pub const FW_COMPONENTS_GET: u16 = 0x1202;
    pub const MODEL_ID_GET: u16 = 0x1206;

    pub const PAIRED_SIZE_GET: u16 = 0x1400;
    pub const PAIRED_DEVICE_GET: u16 = 0x1401;
    pub const PAIRED_CONNECT: u16 = 0x1402;
    pub const PAIRED_DISCONNECT: u16 = 0x1403;
    pub const PAIRED_STATUS_GET: u16 = 0x1404;
    pub const PAIRED_DELETE: u16 = 0x1405;
    pub const PAIRED_DELETE_ALL: u16 = 0x1406;
    pub const PAIRED_OWN_INDEX_GET: u16 = 0x1407;
    pub const PAIRED_MAX_CONNECTIONS_GET: u16 = 0x1409;

    pub const TOUCH_LOCK_SET: u16 = 0x1606;
    pub const TOUCH_LOCK_GET: u16 = 0x1607;

    pub const TRANSPARENCY_MODE_SET: u16 = 0x1800;
    pub const TRANSPARENCY_MODE_GET: u16 = 0x1801;

    pub const ANC_SET: u16 = 0x1a00;
    pub const ANC_GET: u16 = 0x1a01;
    pub const TRANSPARENCY_SET: u16 = 0x1a02;
    pub const TRANSPARENCY_GET: u16 = 0x1a03;
    pub const ANC_STATUS_SET: u16 = 0x1a04;
    pub const ANC_STATUS_GET: u16 = 0x1a05;

    pub const CORE_API_VERSION_GET: u16 = 0x0000;
    pub const CORE_SERIAL_GET: u16 = 0x0003;
}
