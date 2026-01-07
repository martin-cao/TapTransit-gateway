use crate::proto::{decode_frame, encode_frame, Frame, FrameError, FRAME_HEADER, FRAME_VERSION};
use crate::serial::{
    card_detected_from_frame, card_write_result_from_frame, CardAck, CardDetected, CardWriteRequest,
    CardWriteResult,
};
use std::sync::mpsc::Sender;

/// 帧读取器：逐字节组装完整帧。
pub struct FrameReader {
    buffer: Vec<u8>,
    expected_len: Option<usize>,
}

impl FrameReader {
    /// 创建新的帧读取器。
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(256),
            expected_len: None,
        }
    }

    /// 推入一个字节，若解析完成则返回帧或错误。
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

    /// 重置内部状态。
    fn reset(&mut self) {
        self.buffer.clear();
        self.expected_len = None;
    }
}

/// 将帧编码为字节数组。
pub fn frame_to_bytes(frame: &Frame) -> Vec<u8> {
    encode_frame(frame)
}

/// 串口帧解码器（刷卡事件 + 写卡结果）。
pub struct SerialFrameCodec {
    reader: FrameReader,
}

impl SerialFrameCodec {
    /// 创建解码器。
    pub fn new() -> Self {
        Self {
            reader: FrameReader::new(),
        }
    }

    /// 推入一个字节并尝试解析为事件。
    pub fn push_byte(&mut self, byte: u8) -> Option<Result<SerialEvent, FrameError>> {
        let result = self.reader.push(byte)?;
        match result {
            Ok(frame) => {
                if let Some(event) = card_detected_from_frame(&frame) {
                    return Some(Ok(SerialEvent::CardDetected(event)));
                }
                if let Some(result) = card_write_result_from_frame(&frame) {
                    return Some(Ok(SerialEvent::CardWriteResult(result)));
                }
                Some(Err(FrameError::BadLength))
            }
            Err(err) => Some(Err(err)),
        }
    }

    /// 将 ACK 编码为字节序列。
    pub fn ack_to_bytes(ack: &CardAck) -> Vec<u8> {
        frame_to_bytes(&ack.to_frame())
    }

    /// 将写卡请求编码为字节序列。
    pub fn write_req_to_bytes(req: &CardWriteRequest) -> Vec<u8> {
        frame_to_bytes(&req.to_frame())
    }
}

/// 串口事件类型。
pub enum SerialEvent {
    CardDetected(CardDetected),
    CardWriteResult(CardWriteResult),
}

/// 逐字节喂给解码器，解析出事件并发送到通道。
pub fn push_bytes_to_channel(
    codec: &mut SerialFrameCodec,
    bytes: &[u8],
    card_tx: &Sender<CardDetected>,
    write_result_tx: &Sender<CardWriteResult>,
) {
    for &byte in bytes {
        if let Some(Ok(event)) = codec.push_byte(byte) {
            match event {
                SerialEvent::CardDetected(card) => {
                    let _ = card_tx.send(card);
                }
                SerialEvent::CardWriteResult(result) => {
                    let _ = write_result_tx.send(result);
                }
            }
        }
    }
}
