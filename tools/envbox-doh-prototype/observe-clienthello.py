"""Loopback-only TLS ClientHello observer; no certificates or trust changes.

Usage: python observe-clienthello.py OUTPUT_JSON
The observer intentionally aborts TLS after recording SNI and offered ALPN.
It cannot prove negotiated HTTP/2, certificate validation, or Host DNS absence.
"""
import json
import socket
import sys


def exact(stream, count):
    data = b""
    while len(data) < count:
        part = stream.recv(count - len(data))
        if not part:
            raise EOFError("short TLS record")
        data += part
    return data


def hello(record):
    if record[0] != 1:
        raise ValueError("expected ClientHello")
    body = record[4:]
    offset = 34
    offset += 1 + body[offset]
    offset += 2 + int.from_bytes(body[offset:offset + 2], "big")
    offset += 1 + body[offset]
    limit = offset + 2 + int.from_bytes(body[offset:offset + 2], "big")
    offset += 2
    names, protocols = [], []
    while offset + 4 <= limit:
        kind = int.from_bytes(body[offset:offset + 2], "big")
        length = int.from_bytes(body[offset + 2:offset + 4], "big")
        data = body[offset + 4:offset + 4 + length]
        offset += 4 + length
        cursor = 2
        if kind == 0:
            while cursor + 3 <= len(data):
                name_kind = data[cursor]
                size = int.from_bytes(data[cursor + 1:cursor + 3], "big")
                cursor += 3
                if name_kind == 0:
                    names.append(data[cursor:cursor + size].decode("ascii"))
                cursor += size
        if kind == 16:
            while cursor < len(data):
                size = data[cursor]
                cursor += 1
                protocols.append(data[cursor:cursor + size].decode("ascii"))
                cursor += size
    return {"sni": names, "offered_alpn": protocols, "clienthello_size": len(record)}


observations = []
with socket.socket() as listener:
    listener.bind(("127.0.0.1", 18443))
    listener.listen(2)
    listener.settimeout(20)
    for attempt in range(2):
        try:
            stream, peer = listener.accept()
            with stream:
                stream.settimeout(5)
                header = exact(stream, 5)
                record = exact(stream, int.from_bytes(header[3:5], "big"))
                result = hello(record)
                result["peer"] = list(peer)
                observations.append(result)
                stream.sendall(bytes([21, 3, 3, 0, 2, 2, 40]))
        except Exception as error:
            observations.append({"error": str(error)})
with open(sys.argv[1], "w", encoding="utf-8") as output:
    json.dump(observations, output, indent=2)
