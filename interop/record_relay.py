"""Relays one connection between a Viewer and a server and records both directions.

Usage: python record_relay.py LISTEN_PORT SERVER_HOST SERVER_PORT PREFIX

Point the Viewer at 127.0.0.1:LISTEN_PORT. Writes PREFIX.viewer.bin (what the Viewer sent) and
PREFIX.server.bin (what the server sent) when either side closes.
"""
import socket
import sys
import threading


def pump(source, sink, record):
    while True:
        try:
            data = source.recv(65536)
        except OSError:
            data = b""
        if not data:
            break
        record.extend(data)
        try:
            sink.sendall(data)
        except OSError:
            break
    for s in (source, sink):
        try:
            s.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass


def main(listen_port, host, port, prefix):
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", int(listen_port)))
    listener.listen(1)
    print(f"waiting for a Viewer on 127.0.0.1:{listen_port}", flush=True)
    viewer, _ = listener.accept()
    server = socket.create_connection((host, int(port)))
    from_viewer, from_server = bytearray(), bytearray()
    threads = [
        threading.Thread(target=pump, args=(viewer, server, from_viewer)),
        threading.Thread(target=pump, args=(server, viewer, from_server)),
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    open(prefix + ".viewer.bin", "wb").write(from_viewer)
    open(prefix + ".server.bin", "wb").write(from_server)
    print(f"viewer sent {len(from_viewer)} bytes, server sent {len(from_server)} bytes")


if __name__ == "__main__":
    main(*sys.argv[1:])
