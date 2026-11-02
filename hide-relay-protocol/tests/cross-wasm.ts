import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { ownUntilWorkerExit } from '../../web/e2e/worker-owned.ts';
import { initSync, NoiseSession, decodeQr, encodeQr, encodeClient, decodeClient, encodeServer, decodeServer, frame_connection } from '../web/index.ts';

initSync({ module: await readFile(new URL('../pkg/relay_bg.wasm', import.meta.url)) });
const executable = new URL(`../../target/debug/examples/cross_peer${process.platform === 'win32' ? '.exe' : ''}`, import.meta.url);
// This peer starts no descendants and exits on stdin EOF, including if its
// owner is killed. Keep the repository's exit cleanup for ordinary failures.
const peer = spawn(fileURLToPath(executable), [], { stdio: ['pipe', 'pipe', 'inherit'] });
const ownership = ownUntilWorkerExit(() => { peer.kill(); });
const exited = once(peer, 'exit');
// A hang guard only, not a scheduling delay. The finally block reaps the peer.
const timeout = setTimeout(() => peer.kill(), 30_000);
const lines = createInterface({ input: peer.stdout })[Symbol.asyncIterator]();
let phone: NoiseSession | undefined;
try {
  const line = async (): Promise<string> => {
    const next = await lines.next();
    assert.equal(next.done, false, 'native peer ended before replying');
    return next.value;
  };
  const url = await line();
  const qr = decodeQr(url);
  assert.equal(new URL(url).origin + new URL(url).pathname, 'https://app.withhide.dev/');
  assert.equal(new URL(url).search, '');
  assert.equal(encodeQr(qr), url);
  assert.throws(() => decodeQr(url.replace('#pair=', '?pair=')));
  const hello = { type: 'phone' as const, version: 1 as const, room: qr.room };
  assert.deepEqual(decodeClient(encodeClient(hello)), hello);
  const joined = { type: 'joined' as const, connection: 7, phone_protocol: 1 };
  assert.deepEqual(decodeServer(encodeServer(joined)), joined);
  assert.throws(() => decodeClient('{"type":"close","connection":0}'));
  phone = NoiseSession.initiator(Buffer.from(qr.noise_key, 'base64url'), 7);
  const exchange = async (bytes: Uint8Array): Promise<Buffer> => {
    peer.stdin.write(`${Buffer.from(bytes).toString('base64url')}\n`);
    return Buffer.from(await line(), 'base64url');
  };
  phone.read_handshake(await exchange(phone.write_handshake()));
  assert.equal(phone.is_transport(), true);
  let last: Buffer | undefined;
  for (const plaintext of [Buffer.from('phone credential'), Buffer.alloc(256 * 1024, 42), Buffer.alloc(0)]) {
    for (const frame of phone.send_message(plaintext)) {
      assert.equal(frame_connection(frame), 7);
      assert.ok(frame.length <= 65_536);
      peer.stdin.write(`${Buffer.from(frame).toString('base64url')}\n`);
    }
    let result: Uint8Array | undefined;
    while (result === undefined) {
      last = Buffer.from(await line(), 'base64url');
      result = phone.recv_message(last, 0n);
    }
    assert.deepEqual(Buffer.from(result), plaintext);
  }
  assert.throws(() => phone!.recv_message(last!, 0n), /authentication_failed/);
  assert.throws(() => phone!.send_message(Buffer.alloc(0)), /session_closed/);
  const [code] = await exited;
  assert.equal(code, 0);
  console.log('PASS: TypeScript-driven production WASM exchanges credentials, 256 KiB message and empty payload with native Rust in both directions; replay closes session; QR stays in fragment.');
} finally {
  clearTimeout(timeout);
  phone?.free();
  peer.stdin.end();
  if (peer.exitCode === null && peer.signalCode === null) peer.kill();
  await exited;
  ownership.disown();
}
