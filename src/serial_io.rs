use crate::proto::{decode_frame, encode_frame, Frame, FrameError, FRAME_HEADER, FRAME_VERSION};
use crate::serial::{card_detected_from_frame, CardAck, CardDetected};
use std::sync::mpsc::Sender;

pub struct FrameReader {
    buffer: Vec<u8>,
    expected_len: Option<usize>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(256),
            expected_len: None,
        }
    }

    pub fn push(&mut self, byte: u8) -> Option<Result<Frame, FrameError>> {
        self.buffer.push(byte);

        if self.buffer.len() == 1 && self.buffer[0] != FRAME_HEADER[0] {
            self.reset();
            return None;
        }
        if self.buffer.len() == 2 && self.buffer[1] != FRAME_HEADER[1] {
            self.reset();
            return None;
        }
        if self.buffer.len() == 3 && self.buffer[2] != FRAME_VERSION {
            let err = FrameError::BadVersion;
            self.reset();
            return Some(Err(err));
        }

        if self.buffer.len() == 5 {
            let len = u16::from_le_bytes([self.buffer[3], self.buffer[4]]) as usize;
            self.expected_len = Some(2 + 1 + 2 + len + 2);
        }

        if let Some(expected) = self.expected_len {
            if self.buffer.len() == expected {
                let frame = decode_frame(&self.buffer);
                self.reset();
                return Some(frame);
            }
            if self.buffer.len() > expected {
                let err = FrameError::BadLength;
                self.reset();
                return Some(Err(err));
            }
        }

        None
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.expected_len = None;
    }
}

pub fn frame_to_bytes(frame: &Frame) -> Vec<u8> {
    encode_frame(frame)
}

pub struct CardFrameCodec {
    reader: FrameReader,
}

impl CardFrameCodec {
    pub fn new() -> Self {
        Self {
            reader: FrameReader::new(),
        }
    }

    pub fn push_byte(&mut self, byte: u8) -> Option<Result<CardDetected, FrameError>> {
        let result = self.reader.push(byte)?;
        match result {
            Ok(frame) => match card_detected_from_frame(&frame) {
                Some(event) => Some(Ok(event)),
                None => Some(Err(FrameError::BadLength)),
            },
            Err(err) => Some(Err(err)),
        }
    }

    pub fn ack_to_bytes(ack: &CardAck) -> Vec<u8> {
        frame_to_bytes(&ack.to_frame())
    }
}

pub fn push_bytes_to_channel(
    codec: &mut CardFrameCodec,
    bytes: &[u8],
    card_tx: &Sender<CardDetected>,
) {
    for &byte in bytes {
        if let Some(Ok(card)) = codec.push_byte(byte) {
            let _ = card_tx.send(card);
        }
    }
}
