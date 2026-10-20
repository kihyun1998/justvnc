# Input and clipboard

What a client sends besides update requests, and the two small server messages that are not updates. Encoding: `justvnc_proto::client_message::{key_event, pointer_event, client_cut_text}`. Framing: `ServerMessageReader` (Bell, ServerCutText). Text: `justvnc_proto::CutText`. A minimal host that uses all of it: `justvnc-client/examples/viewer.rs`.

## Rules taken from the specs

- **KeyEvent** carries an X11 keysym, whatever the platform (RFC 6143 §7.5.4). The library takes the keysym as given; choosing it (layouts, dead keys, Ctrl+letter) is the host's job, and the example shows one way to do it. Latin-1 characters are their own keysym; any other Unicode character is `0x01000000` plus its code point.
- **PointerEvent** puts buttons 1 to 8 in bits 0 to 7. A wheel step is a press and release of button 4 (up) or 5 (down), so it is two PointerEvents.
- **Cut text is Latin-1 with LF line ends**, both ways (§7.5.6, §7.6.4). `CutText::from_text` refuses a character Latin-1 cannot hold (`NotLatin1`) instead of replacing it: a silent `?` would corrupt the clipboard without anyone noticing. It turns CRLF into LF because the rule belongs to the protocol, not to each caller; Windows clipboards are CRLF. A lone CR is kept. UTF-8 clipboards need the Extended Clipboard pseudo-encoding, which is not implemented.
- **ServerCutText length** is a peer-chosen U32, checked against `max_cut_text_length` (default 1 MiB, vnc-rs's figure too) before the text is waited for.

## What TigerVNC was seen to do

Probed against TigerVNC 1.13.1 in `interop/tigervnc` (see `docs/development.md`):

- ClientCutText in Latin-1 lands in both X selections, CLIPBOARD and PRIMARY, converted to UTF-8.
- A change to the X clipboard is sent as ServerCutText in Latin-1. Invalid UTF-8 in the selection arrives as `?`.
- A client's ClientCutText is **not** relayed to other VNC clients, so a test cannot check clipboard sending through a second connection. It has to read the X selection (`xsel`).
- It normalises CRLF to LF on its own, so the interop test cannot catch a client that forgets to. The session test can.
- One `xkbbell` arrives as **three** Bell messages.
- PointerEvent moves the cursor Xvnc draws into the framebuffer (top-left 7 pixels above and left of the pointer), but only after the X server has handled the event. The first update after a PointerEvent can still show the old position, and an incremental update request is never answered when nothing changed. The pointer test therefore asks for full updates until the cursor appears, and puts the pointer back at the centre afterwards: the server outlives a test run, and the frame test expects the centre.

## Fixture

`tigervnc-3.8-none-cut-text-and-bell` comes from `interop/record_server_messages.py`, run while `printf 'from-x\n\303\251' | xsel -i -b` and then `xkbbell` executed in the container.
