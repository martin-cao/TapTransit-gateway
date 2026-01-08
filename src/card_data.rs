use crate::model::Direction;

pub const CARD_DATA_LEN: usize = 32;
pub const CARD_DATA_BLOCK_START: u8 = 8;
pub const CARD_DATA_BLOCK_COUNT: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardDataParseError {
    BadLength,
    BadMagic,
    BadVersion,
    BadUidLen,
    BadCrc,
    UnknownStatus,
}

impl CardDataParseError {
    pub fn as_str(&self) -> &'static str {
        match self {
            CardDataParseError::BadLength => "bad_length",
            CardDataParseError::BadMagic => "bad_magic",
            CardDataParseError::BadVersion => "bad_version",
            CardDataParseError::BadUidLen => "bad_uid_len",
            CardDataParseError::BadCrc => "bad_crc",
            CardDataParseError::UnknownStatus => "unknown_status",
        }
    }
}
const MAGIC: [u8; 2] = [0x54, 0x54];
const VERSION: u8 = 0x01;
const EMPTY_ID: u16 = 0xFFFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardStatus {
    Idle,
    InTrip,
    Blocked,
}

impl CardStatus {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(CardStatus::Idle),
            1 => Some(CardStatus::InTrip),
            2 => Some(CardStatus::Blocked),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            CardStatus::Idle => 0,
            CardStatus::InTrip => 1,
            CardStatus::Blocked => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CardStatus::Idle => "idle",
            CardStatus::InTrip => "in_trip",
            CardStatus::Blocked => "blocked",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CardData {
    pub uid: [u8; 4],
    pub balance_cents: u32,
    pub status: CardStatus,
    pub entry_station_id: Option<u16>,
    pub last_route_id: Option<u16>,
    pub last_direction: Option<Direction>,
    pub last_board_station_id: Option<u16>,
    pub last_alight_station_id: Option<u16>,
}

impl CardData {
    pub fn new(uid: [u8; 4]) -> Self {
        Self {
            uid,
            balance_cents: 0,
            status: CardStatus::Idle,
            entry_station_id: None,
            last_route_id: None,
            last_direction: None,
            last_board_station_id: None,
            last_alight_station_id: None,
        }
    }

    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        Self::from_bytes_verbose(data).ok()
    }

    pub fn from_bytes_verbose(data: &[u8]) -> Result<Self, CardDataParseError> {
        if data.len() < CARD_DATA_LEN {
            return Err(CardDataParseError::BadLength);
        }
        if data[0..2] != MAGIC {
            return Err(CardDataParseError::BadMagic);
        }
        if data[2] != VERSION {
            return Err(CardDataParseError::BadVersion);
        }
        if data[3] != 4 {
            return Err(CardDataParseError::BadUidLen);
        }
        let stored_crc = u16::from_le_bytes([data[30], data[31]]);
        let computed_crc = crc16(&data[..30]);
        if stored_crc != computed_crc {
            return Err(CardDataParseError::BadCrc);
        }
        let uid = [data[4], data[5], data[6], data[7]];
        let balance_cents = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        let status = CardStatus::from_u8(data[16]).ok_or(CardDataParseError::UnknownStatus)?;
        let entry_station_id = decode_optional_u16(&data[18..20]);
        let last_route_id = decode_optional_u16(&data[20..22]);
        let last_direction = decode_direction(data[22]);
        let last_board_station_id = decode_optional_u16(&data[24..26]);
        let last_alight_station_id = decode_optional_u16(&data[26..28]);

        Ok(Self {
            uid,
            balance_cents,
            status,
            entry_station_id,
            last_route_id,
            last_direction,
            last_board_station_id,
            last_alight_station_id,
        })
    }

    pub fn to_bytes(&self) -> [u8; CARD_DATA_LEN] {
        let mut out = [0u8; CARD_DATA_LEN];
        out[0..2].copy_from_slice(&MAGIC);
        out[2] = VERSION;
        out[3] = 4;
        out[4..8].copy_from_slice(&self.uid);
        out[12..16].copy_from_slice(&self.balance_cents.to_le_bytes());
        out[16] = self.status.as_u8();
        write_optional_u16(&mut out[18..20], self.entry_station_id);
        write_optional_u16(&mut out[20..22], self.last_route_id);
        out[22] = encode_direction(self.last_direction);
        write_optional_u16(&mut out[24..26], self.last_board_station_id);
        write_optional_u16(&mut out[26..28], self.last_alight_station_id);
        let crc = crc16(&out[..30]);
        out[30..32].copy_from_slice(&crc.to_le_bytes());
        out
    }
}

pub fn decode_uid_hex(input: &str) -> Option<[u8; 4]> {
    if input.len() != 8 {
        return None;
    }
    let bytes = input.as_bytes();
    let mut out = [0u8; 4];
    for (i, chunk) in bytes.chunks(2).enumerate() {
        let hi = hex_val(chunk[0])?;
        let lo = hex_val(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn decode_optional_u16(bytes: &[u8]) -> Option<u16> {
    let value = u16::from_le_bytes([bytes[0], bytes[1]]);
    if value == EMPTY_ID {
        None
    } else {
        Some(value)
    }
}

fn write_optional_u16(out: &mut [u8], value: Option<u16>) {
    let value = value.unwrap_or(EMPTY_ID);
    out.copy_from_slice(&value.to_le_bytes());
}

fn decode_direction(value: u8) -> Option<Direction> {
    match value {
        0 => Some(Direction::Up),
        1 => Some(Direction::Down),
        _ => None,
    }
}

fn encode_direction(value: Option<Direction>) -> u8 {
    match value {
        Some(Direction::Up) => 0,
        Some(Direction::Down) => 1,
        None => 0xFF,
    }
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
