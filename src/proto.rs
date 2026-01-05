#[derive(Clone, Debug)]
pub struct Frame {
    pub msg_type: u8,
    pub flags: u8,
    pub payload: Vec<u8>,
}

pub const FRAME_HEADER: [u8; 2] = [0xAA, 0x55];
pub const FRAME_VERSION: u8 = 0x01;

pub const MSG_CARD_DETECTED: u8 = 0x01;
pub const MSG_CARD_ACK: u8 = 0x02;
pub const MSG_SET_ROUTE_INFO: u8 = 0x03;
pub const MSG_HEARTBEAT: u8 = 0x04;
pub const MSG_ERROR_REPORT: u8 = 0x05;

#[derive(Clone, Debug)]
pub enum FrameError {
    TooShort,
    BadHeader,
    BadVersion,
    BadLength,
    BadChecksum,
}

pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + 1 + 2 + 1 + 1 + frame.payload.len() + 2);
    out.extend_from_slice(&FRAME_HEADER);
    out.push(FRAME_VERSION);
    let length = (1 + 1 + frame.payload.len()) as u16;
    out.extend_from_slice(&length.to_le_bytes());
    out.push(frame.msg_type);
    out.push(frame.flags);
    out.extend_from_slice(&frame.payload);
    let checksum = checksum16(&out[2..]);
    out.extend_from_slice(&checksum.to_le_bytes());
    out
}

pub fn decode_frame(data: &[u8]) -> Result<Frame, FrameError> {
    if data.len() < 2 + 1 + 2 + 1 + 1 + 2 {
        return Err(FrameError::TooShort);
    }
    if data[0..2] != FRAME_HEADER {
        return Err(FrameError::BadHeader);
    }
    if data[2] != FRAME_VERSION {
        return Err(FrameError::BadVersion);
    }
    let length = u16::from_le_bytes([data[3], data[4]]) as usize;
    let expected = 2 + 1 + 2 + length + 2;
    if data.len() < expected {
        return Err(FrameError::BadLength);
    }
    let checksum = u16::from_le_bytes([data[expected - 2], data[expected - 1]]);
    let computed = checksum16(&data[2..expected - 2]);
    if checksum != computed {
        return Err(FrameError::BadChecksum);
    }
    let msg_type = data[5];
    let flags = data[6];
    let payload = data[7..expected - 2].to_vec();
    Ok(Frame {
        msg_type,
        flags,
        payload,
    })
}

fn checksum16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |acc, b| acc.wrapping_add(*b as u16))
}
