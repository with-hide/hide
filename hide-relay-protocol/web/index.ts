// Import after build-relay-wasm.sh. Vite includes relay_bg.wasm as a local asset;
// init accepts its same-origin URL or bytes, never a third-party crypto script.
export { default, initSync, NoiseSession, encode_frame, frame_connection, frame_kind, frame_payload } from '../pkg/relay.js';
import { client_message, server_message, decode_qr, encode_qr } from '../pkg/relay.js';

export interface DeviceHello {
  version: 1;
  room: string;
  device_key: string;
  phone_protocol: number;
}
export type ClientMessage =
  | { type: 'device'; hello: DeviceHello }
  | { type: 'proof'; signature: string }
  | { type: 'phone'; version: 1; room: string }
  | { type: 'close'; connection: number };
export type Refusal = 'invalid_message' | 'invalid_proof' | 'unsupported_version' |
  'device_offline' | 'room_occupied' | 'connection_limit' | 'room_limit' |
  'rate_limit' | 'frame_limit' | 'daily_limit' | 'timeout';
export type ServerMessage =
  | { type: 'challenge'; challenge: { nonce: string } }
  | { type: 'registered' }
  | { type: 'joined'; connection: number; phone_protocol: number }
  | { type: 'peer_joined' | 'peer_left'; connection: number }
  | { type: 'refused'; reason: Refusal };
export interface PairingQr {
  v: 2;
  device_key: string;
  noise_key: string;
  room: string;
  code: string;
}

export const encodeClient = (value: ClientMessage): string => client_message(JSON.stringify(value));
export const decodeClient = (text: string): ClientMessage => JSON.parse(client_message(text)) as ClientMessage;
export const encodeServer = (value: ServerMessage): string => server_message(JSON.stringify(value));
export const decodeServer = (text: string): ServerMessage => JSON.parse(server_message(text)) as ServerMessage;
export const decodeQr = (url: string): PairingQr => JSON.parse(decode_qr(url)) as PairingQr;
export const encodeQr = (value: PairingQr): string => encode_qr(JSON.stringify(value));
