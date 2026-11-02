# Noise vector provenance

`cacophony-nk.json` is the `Noise_NK_25519_ChaChaPoly_BLAKE2b` entry from [snow's cacophony vectors](https://github.com/mcginty/snow/blob/2fcd3e2dbe3f4a455df5614538aa589c5854062b/tests/vectors/cacophony.txt).
The file SHA-256 is `d5280968f1e4eafb3ece893e678c987f2c9aa6d3e146ffe0fdf6dde2e4bdad75`.
These are public test secrets, not credentials.
The test sends the vector's nonempty handshake payloads through snow directly; the product session API deliberately permits only empty handshake payloads and fixes its own prologue.
The suite checks both handshakes, the handshake hash and four alternating transport messages on native Rust and WASM.
