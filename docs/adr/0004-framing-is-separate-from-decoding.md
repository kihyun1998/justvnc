# Encoding framing is separate from pixel decoding

The protocol crate only frames each rectangle — finds where its bytes end — and never decompresses or produces pixels; decoding lives in its own crate. A Proxy needs only framing, so it carries no zlib or JPEG decoder, and decompression code stays off the attack surface of the program that sits between untrusted Viewers and Targets. Merging the two per encoding would be simpler to write but would make every Proxy pay for decoders it never uses.
