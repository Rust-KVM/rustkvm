use anyhow::{Result, anyhow, bail};

pub const VERSION: u8 = 0x01;

pub const HID_KEY_BUFFER_SIZE: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageType {
    Handshake = 0x01,
    KeyboardReport = 0x02,
    PointerReport = 0x03,
    WheelReport = 0x04,
    KeypressReport = 0x05,
    MouseReport = 0x06,
    KeyboardMacroReport = 0x07,
    CancelKeyboardMacroReport = 0x08,
    KeypressKeepAliveReport = 0x09,
    KeyboardLedState = 0x32,
    KeydownState = 0x33,
    KeyboardMacroState = 0x34,
}

impl MessageType {
    pub fn from_byte(b: u8) -> Result<Self> {
        match b {
            0x01 => Ok(Self::Handshake),
            0x02 => Ok(Self::KeyboardReport),
            0x03 => Ok(Self::PointerReport),
            0x04 => Ok(Self::WheelReport),
            0x05 => Ok(Self::KeypressReport),
            0x06 => Ok(Self::MouseReport),
            0x07 => Ok(Self::KeyboardMacroReport),
            0x08 => Ok(Self::CancelKeyboardMacroReport),
            0x09 => Ok(Self::KeypressKeepAliveReport),
            0x32 => Ok(Self::KeyboardLedState),
            0x33 => Ok(Self::KeydownState),
            0x34 => Ok(Self::KeyboardMacroState),
            _ => Err(anyhow!("unknown HID RPC message type: 0x{b:02x}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub r#type: MessageType,
    pub data: Vec<u8>,
}

impl Message {
    pub fn new(ty: MessageType, data: Vec<u8>) -> Self {
        Self { r#type: ty, data }
    }

    pub fn unmarshal(buf: &[u8]) -> Result<Self> {
        let &[ty, ref rest @ ..] = buf else {
            bail!("invalid data length: {}", buf.len());
        };
        Ok(Self { r#type: MessageType::from_byte(ty)?, data: rest.to_vec() })
    }

    pub fn marshal(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() + 1);
        out.push(self.r#type as u8);
        out.extend_from_slice(&self.data);
        out
    }
}

pub fn new_handshake_message() -> Message {
    Message::new(MessageType::Handshake, vec![VERSION])
}

pub fn new_keyboard_report_message(modifier: u8, keys: &[u8]) -> Message {
    let mut data = Vec::with_capacity(1 + keys.len());
    data.push(modifier);
    data.extend_from_slice(keys);
    Message::new(MessageType::KeyboardReport, data)
}

pub fn new_keyboard_led_message(state_byte: u8) -> Message {
    Message::new(MessageType::KeyboardLedState, vec![state_byte])
}

pub fn new_keydown_state_message(modifier: u8, keys: &[u8]) -> Message {
    let mut data = Vec::with_capacity(1 + keys.len());
    data.push(modifier);
    data.extend_from_slice(keys);
    Message::new(MessageType::KeydownState, data)
}

pub fn new_keyboard_macro_state_message(state: bool, is_paste: bool) -> Message {
    Message::new(MessageType::KeyboardMacroState, vec![u8::from(state), u8::from(is_paste)])
}

pub fn new_pointer_report_message(x: i32, y: i32, button: u8) -> Message {
    let mut data = Vec::with_capacity(9);
    data.extend_from_slice(&x.to_be_bytes());
    data.extend_from_slice(&y.to_be_bytes());
    data.push(button);
    Message::new(MessageType::PointerReport, data)
}

pub fn new_mouse_report_message(dx: i8, dy: i8, button: u8) -> Message {
    Message::new(MessageType::MouseReport, vec![dx as u8, dy as u8, button])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeypressReport {
    pub key: u8,
    pub press: bool,
}

#[derive(Debug)]
pub struct KeyboardReport<'a> {
    pub modifier: u8,
    pub keys: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerReport {
    pub x: i32,
    pub y: i32,
    pub button: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseReport {
    pub dx: i8,
    pub dy: i8,
    pub button: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardMacroStep {
    pub modifier: u8,
    pub keys: [u8; HID_KEY_BUFFER_SIZE],
    pub delay: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardMacroReport {
    pub is_paste: bool,
    pub step_count: u32,
    pub steps: Vec<KeyboardMacroStep>,
}

impl Message {
    pub fn keypress_report(&self) -> Result<KeypressReport> {
        if self.r#type != MessageType::KeypressReport {
            bail!("invalid message type: {:?}", self.r#type);
        }
        let &[key, press, ..] = self.data.as_slice() else {
            bail!("keypress payload too short: {}", self.data.len());
        };
        Ok(KeypressReport { key, press: press == 1 })
    }

    pub fn keyboard_report(&self) -> Result<KeyboardReport<'_>> {
        if self.r#type != MessageType::KeyboardReport {
            bail!("invalid message type: {:?}", self.r#type);
        }
        let [modifier, keys @ ..] = self.data.as_slice() else {
            bail!("keyboard payload too short: {}", self.data.len());
        };
        Ok(KeyboardReport { modifier: *modifier, keys })
    }

    pub fn pointer_report(&self) -> Result<PointerReport> {
        if self.r#type != MessageType::PointerReport {
            bail!("invalid message type: {:?}", self.r#type);
        }
        let Ok(payload): Result<&[u8; 9], _> = self.data.as_slice().try_into() else {
            bail!("pointer payload length must be 9, got {}", self.data.len());
        };
        Ok(PointerReport {
            x: i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
            y: i32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
            button: payload[8],
        })
    }

    pub fn mouse_report(&self) -> Result<MouseReport> {
        if self.r#type != MessageType::MouseReport {
            bail!("invalid message type: {:?}", self.r#type);
        }
        let &[dx, dy, button, ..] = self.data.as_slice() else {
            bail!("mouse payload too short: {}", self.data.len());
        };
        Ok(MouseReport { dx: dx as i8, dy: dy as i8, button })
    }

    pub fn keyboard_macro_report(&self) -> Result<KeyboardMacroReport> {
        if self.r#type != MessageType::KeyboardMacroReport {
            bail!("invalid message type: {:?}", self.r#type);
        }
        if self.data.len() < 5 {
            bail!("macro payload too short: {}", self.data.len());
        }
        let is_paste = self.data[0] == 1;
        let step_count =
            u32::from_be_bytes([self.data[1], self.data[2], self.data[3], self.data[4]]);

        let step_size = 1 + HID_KEY_BUFFER_SIZE + 2;
        let expected = 5 + step_count as u64 * step_size as u64;
        if self.data.len() as u64 != expected {
            bail!("invalid length: {}, expected: {expected}", self.data.len());
        }

        let mut steps = Vec::with_capacity(step_count as usize);
        let mut offset = 5;
        for _ in 0..step_count {
            let modifier = self.data[offset];
            let mut keys = [0u8; HID_KEY_BUFFER_SIZE];
            keys.copy_from_slice(&self.data[offset + 1..offset + 1 + HID_KEY_BUFFER_SIZE]);
            let delay_off = offset + 1 + HID_KEY_BUFFER_SIZE;
            let delay = u16::from_be_bytes([self.data[delay_off], self.data[delay_off + 1]]);
            steps.push(KeyboardMacroStep { modifier, keys, delay });
            offset += step_size;
        }

        Ok(KeyboardMacroReport { is_paste, step_count, steps })
    }
}
