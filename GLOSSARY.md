# justvnc

A Rust implementation of the RFB (VNC) protocol, usable as a client, a server, or both at once as a proxy.

## Roles

**Viewer**:
The end-user VNC client application a person looks through (e.g. TightVNC Viewer).
_Avoid_: client (ambiguous with the client role), downstream

**Target**:
The VNC server the Viewer is ultimately trying to reach.
_Avoid_: server (ambiguous with the server role), upstream, node

**Proxy**:
A program that plays the server role toward a Viewer and the client role toward a Target, relaying between them.
_Avoid_: broker, gateway, relay

## Connections

**Viewer leg**:
The connection between a Viewer and a Proxy, where the Proxy plays the server role.
_Avoid_: downstream

**Target leg**:
The connection between a Proxy and a Target, where the Proxy plays the client role.
_Avoid_: upstream

## Messages

**Framing**:
Finding where one message, or one rectangle of a framebuffer update, ends in the byte stream, without interpreting its pixels.
_Avoid_: parsing (too broad), decoding

**Decoding**:
Turning a framed rectangle's encoded bytes into pixels.
_Avoid_: parsing, rendering

**Tap**:
The point in a Proxy where the host sees each framed message before it is forwarded, and chooses to forward, drop, replace it, or end the session.
_Avoid_: hook, filter, interceptor

**Recording**:
The framed messages that crossed a session after its handshake, each with its direction and a time, kept to be replayed later.
_Avoid_: capture (that is screen capture), log, video
