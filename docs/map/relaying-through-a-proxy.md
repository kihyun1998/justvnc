# Relaying through a Proxy

How `justvnc_proxy::ProxySession` takes a Viewer to a Target and relays between them (ADR-0003). The viewer leg is `justvnc_proto::server::ServerHandshake`, the target leg `justvnc_proto::client::ClientHandshake`; after both, each direction is framed with `ClientMessageReader` and `ServerMessageReader` and forwarded byte for byte. Nothing is decoded: `justvnc-proxy` depends on `justvnc-proto` only, and CI fails if `cargo tree` shows flate2, miniz_oxide, zune-jpeg or `justvnc-decode` under it.

## The handshake

1. The Viewer is offered `ProxyConfig::security` only.
2. VNC authentication is reported as `Event::Authenticate`, and the host decides with `accept` or `reject`, as in the server role. Security type None waits for the host too (`ServerHandshake::confirming_none`), so both paths reach the same point.
3. `Event::TargetNeeded`: only now does the host connect to the Target and call `connect_target` with the Target's own password. `connect_target` does nothing before this event; a host that calls it early must not get a Viewer through security it never decided (`never_tells_the_viewer_it_passed_before_the_host_decided`).
4. The Viewer's SecurityResult is held until the Target's ServerInit arrives. That is what lets a Target failure reach the Viewer as a failed connection: under 3.8 the Viewer is told "Connection to the Target failed"; before 3.8, after None, there is no SecurityResult to fail, so the host just closes. The Target's own reason is not passed on, since it describes the Target, not the Viewer's credentials. The host refuses a Target it cannot reach with `refuse(reason)`, and that reason is passed on.
5. The Viewer gets the Target's ServerInit unchanged.

The Target is sent ClientInit with `ProxyConfig::shared` (default true, so other users of the Target stay connected), because the Viewer's own ClientInit comes only after its SecurityResult, which waits for the Target.

## Narrowing SetEncodings

The Viewer's SetEncodings is rewritten to the encodings the Proxy can frame, in the Viewer's order: Raw, CopyRect, Tight, ZRLE, DesktopSize, LastRect, and Tight's quality (-32 to -23) and compression (-256 to -247) levels. The levels change only what is inside a Tight rectangle, which frames from its own lengths. The comment left on #13 from #11 said the quality level was dropped; that was wrong, and it is kept. Everything else is dropped: TightVNC Viewer 2.8.85's list loses RRE (5), Hextile (2), PointerPos (-232) and Cursor (-239).

## Framing the Target in the Viewer's terms

The target-leg reader frames in the pixel format the Viewer asked for and with LastRect only if the Viewer offered it, because that is what the Target encodes for. Both change with the Viewer's messages: `ServerMessageReader::set_pixel_format` and `set_last_rect` take effect from the next message on. rfbproto has the client send SetPixelFormat only with no update request outstanding, so no update is under way at that point; the deferral keeps a Viewer that breaks the rule from misframing the update already in flight. It cannot help when that update's header has arrived but its first rectangle has not, because the reader reads the header again then.

A SetPixelFormat that is colour-mapped or not valid is refused (`Error::UnsupportedPixelFormat`) and not forwarded: a colour-mapped Target would send SetColourMapEntries, which is not framed.

## The Tap

The host's `Tap` sees each framed message after narrowing has been decided but before anything is forwarded, and answers `Verdict::Forward`, `Drop`, `Replace(message)` or `End(reason)`. A replacement from the Viewer side goes through the same handling as a message the Viewer sent: a SetEncodings is narrowed and sets LastRect on the target-leg reader, and a SetPixelFormat is checked and sets the reader's format. Otherwise a Tap that took LastRect out would leave the reader expecting it, and the stream would desynchronise.

**Maintainer's call (#14):** the pieces of a FramebufferUpdate (rectangles, copies, DesktopSize, the update's end) are shown to `Tap::target_update`, which can only forward them or end the session. Dropping or replacing one would mean holding the whole update to rewrite its count, delaying the screen by an update's worth and keeping up to the limits in memory, and replacing a Tight or ZRLE rectangle would break the zlib stream the Viewer carries across rectangles. The maintainer was shown that option, and not showing updates to the Tap at all. Bell and ServerCutText from the Target, and every message from the Viewer, can be dropped or replaced.

## Lifecycle

The session reports `ViewerAuthenticated` (after `accept`, or None chosen), `TargetNeeded`, `TargetConnected { desktop }` (the Target's ServerInit) and, once and last, `Closed { reason }`. The reason is `ViewerClosed` or `TargetClosed`, which only the host can know and reports with `viewer_closed` and `target_closed`; `Refused(reason)` after `reject` or `refuse`; `Ended(reason)` from the Tap; or `Failed(error)`. The tokio driver tells the Tap each event through `Tap::event`, for audit logs, and returns `Ended::ByTap(reason)` when the Tap ends a session.

## Passwords

**Maintainer's call (#13):** `Password` overwrites its bytes with zeros when dropped (the `zeroize` crate), because the Proxy holds the Target's real password for a whole session. The maintainer was shown keeping #3's decision to leave it out, and told that zeroing on drop does not reach copies a `Vec` left behind when it grew. The comment that raised it said zeroize was already a dependency through `des`; it was not, so this added it. The test checks `ZeroizeOnDrop` only: reading freed memory to watch the zeros would be undefined behaviour.

## The tokio driver

`justvnc_proxy::tokio::serve` runs one Viewer: the viewer leg, then `connect`, then both legs in a `select!` over the two reads, which are cancel-safe; writes happen after the select, so none is ever cancelled.

`authenticate` and `connect` are plain closures returning futures, with the challenge and response passed by value. The first version took async closures over references (`AsyncFnMut(&[u8; 16], &[u8; 16])`); a closure that borrowed captured state then made `serve`'s future not `Send`, so it could not go to `tokio::spawn`, which is how a host serving many Viewers runs it. The server role's `accept` changed the same way. `serve_can_be_spawned` and `accept_can_be_spawned` compile only while that holds.

## Recordings

The relay tests join a real Viewer's recording (TightVNC Viewer 2.8.85, from #11) to real servers' recordings (TigerVNC 1.13.1, from the client role's fixtures). What the Proxy sends each way must equal what the other side recorded: the Target's bytes reach the Viewer unchanged, the Viewer's reach the Target with SetEncodings narrowed, and the target-leg handshake equals the recorder's, whose VNC authentication response came from pycryptodome's DES and was accepted by TigerVNC. CI also runs justvnc's client role through the Proxy to TigerVNC (`crates/justvnc-proxy/tests/interop.rs`): the frame test, a Target password injected for a Viewer with a token, and clipboard text.

No session with the Targets ACRA connects to has been recorded yet; that is a later issue (maintainer's call, #13).
