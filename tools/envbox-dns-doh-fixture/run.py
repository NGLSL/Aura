"""Local DoH prototype matrix. Does not install trust or mutate host DNS.

Usage: python run.py EXE64 TRAP64 [EXE32 TRAP32]
Requires cryptography and h2 in an explicitly isolated Python package directory.
Each native fixture starts fresh and loads only its own-process instrumentation.
"""
import datetime
import ipaddress
import json
import pathlib
import re
import socket
import ssl
import subprocess
import sys
import threading
import time
import uuid

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import AuthorityInformationAccessOID, ExtendedKeyUsageOID, NameOID
from h2.config import H2Configuration
from h2.connection import H2Connection
from h2.events import DataReceived, RequestReceived, StreamEnded

OUT = pathlib.Path("target") / ("doh-cert-" + str(uuid.uuid4()))
OUT.mkdir(parents=True)
NOW = datetime.datetime.now(datetime.timezone.utc)
CANARY = socket.socket()
CANARY.bind(("127.0.0.1", 0))
CANARY.listen(8)
CANARY.settimeout(.1)
CANARY_URL = f"http://127.0.0.1:{CANARY.getsockname()[1]}/must-not-fetch"
CANARY_COUNT = 0
STOP = threading.Event()


def canary():
    global CANARY_COUNT
    while not STOP.is_set():
        try:
            client, _ = CANARY.accept()
        except socket.timeout:
            continue
        CANARY_COUNT += 1
        client.close()


def cert(label, *, issuer=None, private=None, ca=False, expired=False, wrong_eku=False):
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, label if ca else "fixture.test")])
    issuer_key, issuer_cert = issuer if issuer else (key, None)
    builder = (x509.CertificateBuilder().subject_name(name)
               .issuer_name(issuer_cert.subject if issuer_cert else name)
               .public_key(key.public_key()).serial_number(x509.random_serial_number())
               .not_valid_before(NOW - datetime.timedelta(days=2))
               .not_valid_after(NOW - datetime.timedelta(days=1) if expired else NOW + datetime.timedelta(days=2))
               .add_extension(x509.BasicConstraints(ca=ca, path_length=2 if ca else None), True)
               .add_extension(x509.KeyUsage(True, False, not ca, False, False, ca, ca, False, False), True)
               .add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()), False)
               .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(issuer_key.public_key()), False)
               .add_extension(x509.AuthorityInformationAccess([x509.AccessDescription(
                   AuthorityInformationAccessOID.CA_ISSUERS, x509.UniformResourceIdentifier(CANARY_URL))]), False)
               .add_extension(x509.CRLDistributionPoints([x509.DistributionPoint(
                   [x509.UniformResourceIdentifier(CANARY_URL)], None, None, None)]), False))
    if not ca:
        builder = builder.add_extension(x509.SubjectAlternativeName([
            x509.DNSName("fixture.test"), x509.IPAddress(ipaddress.ip_address("127.0.0.1")),
            x509.IPAddress(ipaddress.ip_address("::1"))]), False)
    if wrong_eku or not ca:
        builder = builder.add_extension(x509.ExtendedKeyUsage([
            ExtendedKeyUsageOID.CLIENT_AUTH if wrong_eku else ExtendedKeyUsageOID.SERVER_AUTH]), False)
    certificate = builder.sign(issuer_key, hashes.SHA256())
    (OUT / (label + ".der")).write_bytes(certificate.public_bytes(serialization.Encoding.DER))
    (OUT / (label + ".pem")).write_bytes(certificate.public_bytes(serialization.Encoding.PEM))
    (OUT / (label + ".key")).write_bytes(key.private_bytes(serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    return key, certificate


ROOT = cert("root", ca=True)
OTHER = cert("other", ca=True)
BADROOT = cert("bad-root", ca=True, wrong_eku=True)
INTERMEDIATE = cert("intermediate", ca=True, issuer=ROOT)
LEAF = cert("leaf", issuer=ROOT)
cert("expired", issuer=ROOT, expired=True)
cert("untrusted", issuer=OTHER)
cert("bad-root-leaf", issuer=BADROOT)
CHAINLEAF = cert("chain-leaf", issuer=INTERMEDIATE)
(OUT / "chain.pem").write_bytes((OUT / "chain-leaf.pem").read_bytes() + (OUT / "intermediate.pem").read_bytes())


def crl(label, issuer, revoked=(), stale=False):
    key, authority = issuer
    builder = (x509.CertificateRevocationListBuilder().issuer_name(authority.subject)
               .last_update(NOW - datetime.timedelta(days=2) if stale else NOW - datetime.timedelta(hours=1))
               .next_update(NOW - datetime.timedelta(days=1) if stale else NOW + datetime.timedelta(days=1))
               .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(key.public_key()), False))
    for item in revoked:
        builder = builder.add_revoked_certificate(x509.RevokedCertificateBuilder().serial_number(item.serial_number)
            .revocation_date(NOW - datetime.timedelta(hours=1)).build())
    (OUT / (label + ".der")).write_bytes(builder.sign(key, hashes.SHA256()).public_bytes(serialization.Encoding.DER))


crl("root-crl", ROOT)
crl("other-crl", OTHER)
crl("stale-crl", ROOT, stale=True)
crl("revoked-leaf-crl", ROOT, [LEAF[1]])
crl("revoked-ca-crl", ROOT, [INTERMEDIATE[1]])
crl("ca-crl", INTERMEDIATE)
crl("bad-root-crl", BADROOT)
tampered_crl = bytearray((OUT / "root-crl.der").read_bytes())
tampered_crl[-1] ^= 1  # Preserve ASN.1, invalidate only the signature.
(OUT / "bad-signature-crl.der").write_bytes(tampered_crl)


def case(executable, trap, label, *, certificate="leaf", key=None, identity="fixture.test",
         roots="root", crls="root-crl", ca="", deny="", behavior="answer", protocol="h2",
         expected=0, budget=1800, cancel=None, repeats=1, tls12=False, bootstrap="127.0.0.1",
         cached_crls=None):
    address = ipaddress.ip_address(bootstrap)
    listener = socket.socket(socket.AF_INET6 if address.version == 6 else socket.AF_INET)
    listener.bind((bootstrap, 0))
    listener.listen(8)
    listener.settimeout(.1)
    port = listener.getsockname()[1]
    stop = threading.Event()
    observed = {"connections": 0, "requests": 0, "peer": [], "sni": [], "alpn": [], "tls": [], "authority": []}
    baseline_canary = CANARY_COUNT

    def respond(secure, packet, stream_id=None, connection=None):
        body = packet[:2] + b"\x81\x80" + packet[4:]
        if behavior in ("maximum", "stream-oversize"):
            body += bytes((65535 if behavior == "maximum" else 65536) - len(body))
        status = "302" if behavior == "redirect" else "500" if behavior == "http-error" else "200"
        media = "text/plain" if behavior == "bad-media" else "Application/Dns-Message; charset=binary"
        fields = [("content-type", media), ("content-length", str(65536 if behavior == "oversize" else len(body)))]
        if behavior == "stream-oversize":
            fields = [("content-type", media)]
        if behavior == "redirect":
            fields.append(("location", CANARY_URL))
        if behavior == "encoding":
            fields.append(("content-encoding", "gzip"))
        if connection:
            connection.send_headers(stream_id, [(":status", status), *fields])
            secure.sendall(connection.data_to_send())
            if behavior in ("read-stall", "oversize"):
                time.sleep(1)
                return
            offset = 0
            while offset < len(body):
                count = min(connection.local_flow_control_window(stream_id),
                            connection.max_outbound_frame_size, len(body) - offset)
                if count == 0:
                    incoming = secure.recv(65535)
                    if not incoming:
                        raise EOFError()
                    connection.receive_data(incoming)
                else:
                    connection.send_data(stream_id, body[offset:offset + count], end_stream=offset + count == len(body))
                    offset += count
                secure.sendall(connection.data_to_send())
        else:
            secure.sendall(("HTTP/1.1 " + status + " fixture\r\n" + "\r\n".join(k + ": " + v for k, v in fields) + "\r\n\r\n").encode())
            if behavior == "read-stall":
                time.sleep(1)
            else:
                secure.sendall(body)

    def server():
        try:
            for _ in range(repeats):
                while not stop.is_set():
                    try:
                        client, peer = listener.accept()
                        break
                    except socket.timeout:
                        continue
                else:
                    return
                observed["connections"] += 1
                observed["peer"].append(peer[0])
                with client:
                    client.settimeout(3)
                    if behavior == "handshake-stall":
                        time.sleep(1)
                        continue
                    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
                    context.minimum_version = ssl.TLSVersion.TLSv1_2
                    if tls12:
                        context.maximum_version = ssl.TLSVersion.TLSv1_2
                    context.set_alpn_protocols([protocol])
                    context.set_servername_callback(lambda sock, name, _: observed["sni"].append(name))
                    context.load_cert_chain(str(OUT / (certificate + ".pem")), str(OUT / ((key or certificate) + ".key")))
                    with context.wrap_socket(client, server_side=True) as secure:
                        observed["alpn"].append(secure.selected_alpn_protocol())
                        observed["tls"].append(secure.version())
                        if protocol == "h2":
                            connection = H2Connection(config=H2Configuration(client_side=False, header_encoding="utf-8"))
                            connection.initiate_connection()
                            secure.sendall(connection.data_to_send())
                            bodies = {}
                            completed = False
                            while not completed:
                                incoming = secure.recv(65535)
                                if not incoming:
                                    raise EOFError()
                                for event in connection.receive_data(incoming):
                                    if isinstance(event, RequestReceived):
                                        observed["requests"] += 1
                                        headers = dict(event.headers)
                                        assert headers[":method"] == "POST"
                                        observed["authority"].append(headers[":authority"])
                                        bodies[event.stream_id] = b""
                                    elif isinstance(event, DataReceived):
                                        bodies[event.stream_id] += event.data
                                        connection.acknowledge_received_data(event.flow_controlled_length, event.stream_id)
                                    elif isinstance(event, StreamEnded):
                                        respond(secure, bodies[event.stream_id], event.stream_id, connection)
                                        completed = True
                                pending = connection.data_to_send()
                                if pending:
                                    secure.sendall(pending)
                        else:
                            incoming = b""
                            while b"\r\n\r\n" not in incoming:
                                incoming += secure.recv(65535)
                            head, body = incoming.split(b"\r\n\r\n", 1)
                            lines = head.decode().split("\r\n")
                            assert lines[0] == "POST /dns-query?fixture=1 HTTP/1.1"
                            headers = dict(line.split(": ", 1) for line in lines[1:])
                            length = int(next(value for key, value in headers.items() if key.lower() == "content-length"))
                            while len(body) < length:
                                body += secure.recv(length - len(body))
                            observed["requests"] += 1
                            observed["authority"].append(next(value for key, value in headers.items() if key.lower() == "host"))
                            respond(secure, body)
                        # Keep TLS alive until the client has consumed the
                        # response and completed its own tracked-task cleanup.
                        try:
                            while secure.recv(65535):
                                pass
                        except (OSError, ssl.SSLError):
                            pass
        except (OSError, EOFError, ssl.SSLError) as error:
            observed["closed"] = type(error).__name__
        finally:
            listener.close()

    worker = threading.Thread(target=server)
    worker.start()
    paths = lambda value: ";".join(str(OUT / (name + ".der")) for name in value.split(';') if name)
    authority = f"[{identity}]" if ":" in identity else identity
    url = f"https://{authority}:{port}/dns-query?fixture=1"
    command = [str(executable), "--url", url, "--ip", bootstrap, "--roots", paths(roots),
        "--crls", paths(crls), "--ca", paths(ca), "--deny", paths(deny), "--budget-ms", str(budget),
        "--repeat", str(repeats), "--trap", str(trap)]
    if cancel is not None:
        command += ["--cancel-ms", str(cancel)]
    if cached_crls is not None:
        command += ["--cached-crls", paths(cached_crls)]
    try:
        result = subprocess.run(command, text=True, capture_output=True, timeout=8)
    finally:
        stop.set()
        worker.join(4)
    output = result.stdout.strip()
    print(json.dumps({"arch": pathlib.Path(executable).parent.parent.name, "case": label,
        "stdout": output, "observed": observed, "canary": CANARY_COUNT - baseline_canary}), flush=True)
    assert not worker.is_alive(), label
    assert re.search(r"error=" + str(expected) + r"\b", output), (label, result.returncode, output, result.stderr)
    trap_result = json.loads(output.split("trap=", 1)[1])
    assert trap_result["installed"]
    assert all(api["calls"] == 0 for api in trap_result["apis"][:18]), (label, trap_result)
    assert trap_result["connects_denied"] == 0 and trap_result["extension_denied"] == 0
    assert all(api["calls"] == 0 for api in trap_result["apis"] if api["name"] in ("sendto", "WSASendTo"))
    assert CANARY_COUNT == baseline_canary
    if expected == 0:
        assert result.returncode == 0 and observed["requests"] == repeats, label
        assert all(ipaddress.ip_address(value) == address for value in observed["peer"])
        assert all(value == protocol for value in observed["alpn"])
        assert all(value == f"{authority}:{port}" for value in observed["authority"])
        try:
            ipaddress.ip_address(identity)
            expected_sni = None
        except ValueError:
            expected_sni = identity
        assert all(value == expected_sni for value in observed["sni"])
    elif expected in (6, 7, 8, 9, 10, 11):
        assert observed["requests"] == 0, label


def main():
    # A host policy can deny IPv6 even on loopback. Probe with plain sockets,
    # before any TLS client/trap is involved; an unavailable family is not a pass.
    ipv6_available = False
    try:
        with socket.socket(socket.AF_INET6) as listener, socket.socket(socket.AF_INET6) as client:
            listener.bind(("::1", 0))
            listener.listen(1)
            listener.settimeout(1)
            client.settimeout(1)
            client.connect(listener.getsockname())
            connection, _ = listener.accept()
            connection.close()
        ipv6_available = True
        print(json.dumps({"ipv6_preflight": "available"}), flush=True)
    except OSError as error:
        print(json.dumps({"ipv6_preflight": "unverified", "error": str(error),
                          "winerror": getattr(error, "winerror", None),
                          "unexecuted_cases_per_arch": 5}), flush=True)
    thread = threading.Thread(target=canary)
    thread.start()
    try:
      for index in range(1, len(sys.argv), 2):
        exe, trap = map(pathlib.Path, sys.argv[index:index + 2])
        case(exe, trap, "h2-positive")
        case(exe, trap, "h2-tls12-positive", tls12=True)
        case(exe, trap, "h1-positive", protocol="http/1.1")
        case(exe, trap, "ipv4-ip-identity", identity="127.0.0.1")
        if ipv6_available:
            case(exe, trap, "ipv6-h2-positive", bootstrap="::1")
            case(exe, trap, "ipv6-h1-positive", bootstrap="::1", protocol="http/1.1")
            case(exe, trap, "ipv6-ip-identity", bootstrap="::1", identity="::1")
            case(exe, trap, "ipv6-wrong-ip-identity", bootstrap="::1", identity="::2", expected=9)
            case(exe, trap, "ipv6-read-cancel", bootstrap="::1", behavior="read-stall", cancel=100, expected=2)
        case(exe, trap, "h2-resource", repeats=16)
        case(exe, trap, "untrusted", certificate="untrusted", expected=6)
        case(exe, trap, "expired", certificate="expired", expected=6)
        case(exe, trap, "name", identity="wrong.test", expected=9)
        case(exe, trap, "revoked-ee", crls="revoked-leaf-crl", expected=8)
        case(exe, trap, "empty-crl", crls="", expected=7)
        case(exe, trap, "wrong-issuer-crl", crls="other-crl", expected=7)
        case(exe, trap, "stale-crl", crls="stale-crl", expected=7)
        case(exe, trap, "non-serverauth-root", certificate="bad-root-leaf", roots="bad-root", crls="bad-root-crl", expected=11)
        case(exe, trap, "denied-ee", deny="leaf", expected=10)
        case(exe, trap, "chain-positive", certificate="chain", key="chain-leaf", crls="root-crl;ca-crl")
        case(exe, trap, "revoked-ca", certificate="chain", key="chain-leaf", crls="revoked-ca-crl;ca-crl", expected=8)
        case(exe, trap, "unknown-ca-revocation", certificate="chain", key="chain-leaf", crls="ca-crl", expected=7)
        case(exe, trap, "denied-ca", certificate="chain", key="chain-leaf", crls="root-crl;ca-crl", deny="intermediate", expected=10)
        # Exercise the production retry/standard-verifier branch using explicit
        # fixture candidates. This never seeds or reads the Windows URL cache.
        case(exe, trap, "cached-positive", crls="", cached_crls="root-crl")
        case(exe, trap, "cached-refresh-stale", crls="stale-crl", cached_crls="root-crl")
        case(exe, trap, "cached-miss", crls="", cached_crls="", expected=7)
        case(exe, trap, "cached-wrong-issuer", crls="", cached_crls="other-crl", expected=7)
        case(exe, trap, "cached-stale", crls="", cached_crls="stale-crl", expected=7)
        case(exe, trap, "cached-bad-signature", crls="", cached_crls="bad-signature-crl", expected=5)
        case(exe, trap, "cached-revoked-ee", crls="", cached_crls="revoked-leaf-crl", expected=8)
        case(exe, trap, "cached-chain-positive", certificate="chain", key="chain-leaf", crls="", cached_crls="root-crl;ca-crl")
        case(exe, trap, "cached-revoked-ca", certificate="chain", key="chain-leaf", crls="", cached_crls="revoked-ca-crl;ca-crl", expected=8)
        case(exe, trap, "cached-unknown-ca", certificate="chain", key="chain-leaf", crls="", cached_crls="ca-crl", expected=7)
        case(exe, trap, "cached-cannot-clear-revoked", crls="revoked-leaf-crl", cached_crls="root-crl", expected=8)
        case(exe, trap, "cached-wrong-name", identity="wrong.fixture.test", crls="", cached_crls="root-crl", expected=9)
        case(exe, trap, "cached-untrusted", certificate="untrusted", crls="", cached_crls="root-crl", expected=6)
        case(exe, trap, "redirect", behavior="redirect", expected=12)
        case(exe, trap, "http-non2xx", behavior="http-error", expected=12)
        case(exe, trap, "media", behavior="bad-media", expected=13)
        case(exe, trap, "encoding", behavior="encoding", expected=17)
        case(exe, trap, "oversize", behavior="oversize", expected=14)
        case(exe, trap, "maximum-body", behavior="maximum")
        case(exe, trap, "stream-oversize", behavior="stream-oversize", expected=14)
        case(exe, trap, "handshake-deadline", behavior="handshake-stall", budget=200, expected=3)
        case(exe, trap, "read-deadline", behavior="read-stall", budget=200, expected=3)
        case(exe, trap, "handshake-cancel", behavior="handshake-stall", cancel=100, expected=2)
        case(exe, trap, "read-cancel", behavior="read-stall", cancel=100, expected=2)
      print("PASS executed cases; IPv6=" + ("verified" if ipv6_available else "UNVERIFIED"), OUT, flush=True)
    finally:
        STOP.set()
        thread.join(2)
        CANARY.close()


if __name__ == "__main__":
    main()
