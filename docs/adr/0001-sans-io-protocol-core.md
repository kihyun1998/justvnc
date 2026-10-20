# The protocol core is sans-IO

The RFB protocol core is a state machine that takes bytes in and gives events out; it never owns a socket or a runtime. Tokio support is a thin adapter on top. We chose this over binding the core to Tokio (as vnc-rs does) because the first consumer is a proxy that runs the server role and the client role at once inside a host binary whose runtime justvnc should not dictate, and because handshake and authentication edge cases must be testable deterministically from recorded bytes, without sockets.
