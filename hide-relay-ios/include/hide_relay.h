// The iOS app's way to the relay protocol: hide-relay-protocol behind a C interface (hide-relay-ios/src/lib.rs owns it).
// Every function returns 0 on success, or the code below. A buffer the library hands out is released with hide_relay_buf_free.
#ifndef HIDE_RELAY_H
#define HIDE_RELAY_H

#include <stddef.h>
#include <stdint.h>

typedef struct HideRelaySession HideRelaySession;

typedef struct {
    uint8_t *ptr;
    size_t len;
} HideBuf;

// 1 invalid_message, 2 unsupported_version, 3 size_limit, 4 invalid_key, 5 invalid_signature, 6 invalid_qr,
// 7 random_unavailable, 8 session_closed, 9 invalid_state, 10 authentication_failed, 11 timeout,
// 100 bad_argument, 101 panic.

int32_t hide_relay_session_initiator(const uint8_t *remote_public, uint32_t connection, HideRelaySession **out);
int32_t hide_relay_write_handshake(HideRelaySession *session, HideBuf *out);
int32_t hide_relay_read_handshake(HideRelaySession *session, const uint8_t *frame, size_t len);
// out: the frames to send, in order, each a 4-byte big-endian length and the frame.
int32_t hide_relay_send_message(HideRelaySession *session, const uint8_t *message, size_t len, HideBuf *out);
// out: empty (ptr NULL) while the message is incomplete.
int32_t hide_relay_recv_message(HideRelaySession *session, const uint8_t *frame, size_t len, uint64_t now_ms, HideBuf *out);
int32_t hide_relay_check_timeout(HideRelaySession *session, uint64_t now_ms);
int32_t hide_relay_close(HideRelaySession *session);
void hide_relay_session_free(HideRelaySession *session);
void hide_relay_buf_free(HideBuf buf);
int32_t hide_relay_client_message(const uint8_t *json, size_t len, HideBuf *out);
int32_t hide_relay_server_message(const uint8_t *json, size_t len, HideBuf *out);
int32_t hide_relay_frame_connection(const uint8_t *bytes, size_t len, uint32_t *connection);
// "hide-relay-ios-source:<digest>", a NUL-ended string the release check reads out of the signed app.
const uint8_t *hide_relay_source(void);

#endif
