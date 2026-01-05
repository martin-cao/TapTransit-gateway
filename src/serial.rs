use crate::proto::{Frame, MSG_CARD_ACK, MSG_CARD_DETECTED};

#[derive(Clone, Debug)]
pub struct CardDetected {
    pub card_id: String,
    pub tap_time: u64,
    pub reader_id: u16,
    pub card_data: Vec<u8>,
}

impl CardDetected {
    pub fn to_frame(&self) -> Frame {
        Frame {
            msg_type: MSG_CARD_DETECTED,
            flags: 0,
            payload: encode_card_detected(self),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CardAck {
    pub result: u8,
    pub beep_pattern: u8,
    pub display_code: u8,
    pub write_flag: u8,
    pub write_data: Vec<u8>,
}

impl CardAck {
    pub fn accepted() -> Self {
        Self {
            result: 1,
            beep_pattern: 1,
            display_code: 0,
            write_flag: 0,
            write_data: Vec::new(),
        }
    }

    pub fn rejected() -> Self {
        Self {
            result: 0,
            beep_pattern: 2,
            display_code: 1,
            write_flag: 0,
            write_data: Vec::new(),
        }
    }

    pub fn to_frame(&self) -> Frame {
        Frame {
            msg_type: MSG_CARD_ACK,
            flags: 0,
            payload: encode_card_ack(self),
        }
    }
}

pub fn decode_card_detected(payload: &[u8]) -> Option<CardDetected> {
    let mut cursor = 0;
    let card_id = read_string(payload, &mut cursor)?;
    let tap_time = read_u32(payload, &mut cursor)? as u64;
    let reader_id = read_u16(payload, &mut cursor)?;
    let card_data = read_bytes(payload, &mut cursor)?;
    Some(CardDetected {
        card_id,
        tap_time,
        reader_id,
        card_data,
    })
}

pub fn decode_card_ack(payload: &[u8]) -> Option<CardAck> {
    if payload.len() < 4 {
        return None;
    }
    let result = payload[0];
    let beep_pattern = payload[1];
    let display_code = payload[2];
    let write_flag = payload[3];
    let mut cursor = 4;
    let write_data = read_bytes(payload, &mut cursor)?;
    Some(CardAck {
        result,
        beep_pattern,
        display_code,
        write_flag,
        write_data,
    })
}

pub fn card_detected_from_frame(frame: &Frame) -> Option<CardDetected> {
    if frame.msg_type != MSG_CARD_DETECTED {
        return None;
    }
    decode_card_detected(&frame.payload)
}

pub fn card_ack_from_frame(frame: &Frame) -> Option<CardAck> {
    if frame.msg_type != MSG_CARD_ACK {
        return None;
    }
    decode_card_ack(&frame.payload)
}

fn encode_card_detected(msg: &CardDetected) -> Vec<u8> {
    let mut out = Vec::new();
    write_string(&mut out, &msg.card_id);
    out.extend_from_slice(&(msg.tap_time as u32).to_le_bytes());
    out.extend_from_slice(&msg.reader_id.to_le_bytes());
    write_bytes(&mut out, &msg.card_data);
    out
}

fn encode_card_ack(msg: &CardAck) -> Vec<u8> {
    let mut out = vec![msg.result, msg.beep_pattern, msg.display_code, msg.write_flag];
    write_bytes(&mut out, &msg.write_data);
    out
}

fn write_string(out: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    let len = bytes.len().min(u8::MAX as usize);
    out.push(len as u8);
    out.extend_from_slice(&bytes[..len]);
}

fn write_bytes(out: &mut Vec<u8>, value: &[u8]) {
    let len = value.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&value[..len]);
}

fn read_string(data: &[u8], cursor: &mut usize) -> Option<String> {
    if *cursor >= data.len() {
        return None;
    }
    let len = data[*cursor] as usize;
    *cursor += 1;
    if *cursor + len > data.len() {
        return None;
    }
    let value = String::from_utf8_lossy(&data[*cursor..*cursor + len]).to_string();
    *cursor += len;
    Some(value)
}

fn read_bytes(data: &[u8], cursor: &mut usize) -> Option<Vec<u8>> {
    let len = read_u16(data, cursor)? as usize;
    if *cursor + len > data.len() {
        return None;
    }
    let value = data[*cursor..*cursor + len].to_vec();
    *cursor += len;
    Some(value)
}

fn read_u16(data: &[u8], cursor: &mut usize) -> Option<u16> {
    if *cursor + 2 > data.len() {
        return None;
    }
    let value = u16::from_le_bytes([data[*cursor], data[*cursor + 1]]);
    *cursor += 2;
    Some(value)
}

fn read_u32(data: &[u8], cursor: &mut usize) -> Option<u32> {
    if *cursor + 4 > data.len() {
        return None;
    }
    let value = u32::from_le_bytes([
        data[*cursor],
        data[*cursor + 1],
        data[*cursor + 2],
        data[*cursor + 3],
    ]);
    *cursor += 4;
    Some(value)
}
