# Screen capture and input injection live outside the protocol crates

The server role takes its framebuffer and delivers input events through a trait the caller implements; platform screen capture and OS input injection are a separate, later workspace crate. They are platform-specific and large, and the first consumer (a proxy) needs neither. They are still in scope for justvnc as a whole, so their spec issues are filed up front even though the work comes later.
