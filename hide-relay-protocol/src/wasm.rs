//! Thin browser ABI; cryptography and parsing stay in the native library.
use crate::{
    Error,
    admission::{ClientMessage, ServerMessage},
    frame::Frame,
    noise::Session,
    qr::PairingQr,
};
use wasm_bindgen::prelude::*;

impl From<Error> for JsValue {
    fn from(error: Error) -> Self {
        JsValue::from_str(&error.to_string())
    }
}

#[wasm_bindgen(js_name = NoiseSession)]
pub struct NoiseSession(Session);
#[wasm_bindgen(js_class = NoiseSession)]
impl NoiseSession {
    pub fn initiator(remote_public: &[u8], connection: u32) -> Result<NoiseSession, JsValue> {
        let key = remote_public.try_into().map_err(|_| Error::InvalidKey)?;
        Ok(Self(Session::initiator(key, connection)?))
    }
    pub fn write_handshake(&mut self) -> Result<Vec<u8>, JsValue> {
        Ok(self.0.write_handshake()?)
    }
    pub fn read_handshake(&mut self, frame: &[u8]) -> Result<(), JsValue> {
        Ok(self.0.read_handshake(frame)?)
    }
    #[wasm_bindgen(unchecked_return_type = "Uint8Array[]")]
    pub fn send_message(&mut self, bytes: &[u8]) -> Result<Vec<JsValue>, JsValue> {
        Ok(self
            .0
            .send_message(bytes)?
            .into_iter()
            .map(|frame| js_sys::Uint8Array::from(frame.as_slice()).into())
            .collect())
    }
    pub fn recv_message(&mut self, frame: &[u8], now_ms: u64) -> Result<Option<Vec<u8>>, JsValue> {
        Ok(self.0.recv_message(frame, now_ms)?)
    }
    pub fn check_timeout(&mut self, now_ms: u64) -> Result<(), JsValue> {
        Ok(self.0.check_timeout(now_ms)?)
    }
    pub fn close(&mut self) {
        self.0.close();
    }
}

#[wasm_bindgen]
pub fn decode_qr(url: &str) -> Result<String, JsValue> {
    Ok(serde_json::to_string(&PairingQr::decode(url)?).map_err(|_| Error::InvalidQr)?)
}
#[wasm_bindgen]
pub fn client_message(json: &str) -> Result<String, JsValue> {
    Ok(ClientMessage::decode(json)?.encode()?)
}
#[wasm_bindgen]
pub fn server_message(json: &str) -> Result<String, JsValue> {
    Ok(ServerMessage::decode(json)?.encode()?)
}
#[wasm_bindgen]
pub fn frame_connection(bytes: &[u8]) -> Result<u32, JsValue> {
    Ok(Frame::decode(bytes)?.connection)
}
