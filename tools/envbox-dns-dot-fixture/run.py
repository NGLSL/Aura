"""Standalone local-only TLS fixture; certificates remain in a UUID target folder.

Run with an isolated Python containing cryptography, then pass the native fixture
executable(s). No Windows certificate store or host DNS configuration is changed.
"""
import datetime
import ipaddress
import pathlib
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time
import uuid

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

OUT = pathlib.Path("target") / ("dot-cert-" + str(uuid.uuid4()))
OUT.mkdir(parents=True)
NOW = datetime.datetime.now(datetime.timezone.utc)


def authority(label):
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, label)])
    cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
            .public_key(key.public_key()).serial_number(x509.random_serial_number())
            .not_valid_before(NOW - datetime.timedelta(days=1))
            .not_valid_after(NOW + datetime.timedelta(days=7))
            .add_extension(x509.BasicConstraints(ca=True, path_length=0), True)
            .add_extension(x509.KeyUsage(True, False, False, False, False, True, True, False, False), True)
            .add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()), False)
            .sign(key, hashes.SHA256()))
    (OUT / (label + ".der")).write_bytes(cert.public_bytes(serialization.Encoding.DER))
    return key, cert


CA_KEY, CA = authority("fixture-root")
OTHER_KEY, OTHER = authority("other-root")


def leaf(label, expired=False, issuer=CA, key=CA_KEY):
    private = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    cert = (x509.CertificateBuilder()
            .subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "fixture.test")]))
            .issuer_name(issuer.subject).public_key(private.public_key())
            .serial_number(x509.random_serial_number())
            .not_valid_before(NOW - datetime.timedelta(days=2))
            .not_valid_after(NOW - datetime.timedelta(days=1) if expired else NOW + datetime.timedelta(days=2))
            .add_extension(x509.BasicConstraints(ca=False, path_length=None), True)
            .add_extension(x509.SubjectAlternativeName([x509.DNSName("fixture.test"),
                            x509.IPAddress(ipaddress.ip_address("127.0.0.1"))]), False)
            .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), False)
            .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(key.public_key()), False)
            .sign(key, hashes.SHA256()))
    (OUT / (label + ".pem")).write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    (OUT / (label + ".key")).write_bytes(private.private_bytes(serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
    return cert


VALID = leaf("valid")
leaf("expired", expired=True)
leaf("untrusted", issuer=OTHER, key=OTHER_KEY)


def crl(label, revoked=False):
    builder = (x509.CertificateRevocationListBuilder().issuer_name(CA.subject)
               .last_update(NOW - datetime.timedelta(hours=1))
               .next_update(NOW + datetime.timedelta(days=1))
               .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(CA_KEY.public_key()), False))
    if revoked:
        builder = builder.add_revoked_certificate(x509.RevokedCertificateBuilder()
            .serial_number(VALID.serial_number).revocation_date(NOW - datetime.timedelta(minutes=1)).build())
    (OUT / (label + ".der")).write_bytes(builder.sign(CA_KEY, hashes.SHA256()).public_bytes(serialization.Encoding.DER))


crl("good-crl")
crl("revoked-crl", True)


def exact(stream, length):
    result = b""
    while len(result) < length:
        data = stream.recv(length - len(result))
        if not data:
            raise EOFError()
        result += data
    return result


def run(executable, label, cert="valid", identity="fixture.test", crl_file="good-crl",
        behavior="answer", expected=0, budget=1800, cancel=-1, repeats=1):
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    listener.settimeout(4)
    port = listener.getsockname()[1]
    observed = {"dns": 0, "exception": ""}

    def server():
        try:
          for _ in range(repeats):
            client, _ = listener.accept()
            with client:
                client.settimeout(4)
                if behavior == "handshake-stall":
                    time.sleep(1)
                    return
                context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
                context.minimum_version = ssl.TLSVersion.TLSv1_2
                context.maximum_version = ssl.TLSVersion.TLSv1_2
                context.load_cert_chain(str(OUT / (cert + ".pem")), str(OUT / (cert + ".key")))
                with context.wrap_socket(client, server_side=True) as secure:
                    packet = exact(secure, struct.unpack("!H", exact(secure, 2))[0])
                    observed["dns"] += 1
                    if behavior == "read-stall":
                        time.sleep(1)
                        return
                    # Keep the arbitrary HTTPS QTYPE question, return NODATA.
                    answer = packet[:2] + b"\x81\x80" + packet[4:]
                    if behavior == "maximum":
                        answer += bytes(65535 - len(answer))
                    frame = struct.pack("!H", len(answer)) + answer
                    if behavior == "truncated":
                        secure.sendall(frame[:5])
                    elif behavior == "multi":
                        secure.sendall(frame + frame)
                    elif behavior == "oversize":
                        secure.sendall(b"\xff\xff")  # legal prefix; EOF body is failure
                    else:
                        # Fragment framing and DNS body across TLS records.
                        for part in (frame[:1], frame[1:7], frame[7:]):
                            secure.sendall(part)
                            time.sleep(.01)
        except (OSError, EOFError, ssl.SSLError) as error:
            observed["exception"] = type(error).__name__
        finally:
            listener.close()

    worker = threading.Thread(target=server)
    worker.start()
    command = [str(executable), str(OUT / "fixture-root.der"),
               str(OUT / (crl_file + ".der")) if crl_file else "-", "127.0.0.1", str(port),
               identity, str(budget), label, str(cancel)]
    if repeats > 1:
        command.append(str(repeats))
    completed = subprocess.run(command, capture_output=True, text=True, timeout=5)
    worker.join(5)
    output = completed.stdout.strip()
    print(pathlib.Path(executable).parent.parent.name, output, observed, flush=True)
    if "error=" + str(expected) + " " not in output or worker.is_alive():
        raise AssertionError((label, completed.returncode, output, completed.stderr))
    if expected == 0 and (completed.returncode != 0 or observed["dns"] != repeats):
        raise AssertionError((label, "positive exchange missing"))
    if expected in (6, 7, 8, 9) and observed["dns"]:
        raise AssertionError((label, "DNS sent before certificate verification"))


def main():
  for exe in sys.argv[1:]:
    run(exe, "fragmented-success")
    run(exe, "ip-san-success", identity="127.0.0.1")
    run(exe, "maximum-packet", behavior="maximum")
    run(exe, "connections-resource-bound", repeats=16)
    run(exe, "untrusted", cert="untrusted", expected=6)
    run(exe, "expired", cert="expired", expected=6)
    run(exe, "wrong-name", identity="wrong.test", expected=9)
    run(exe, "revoked", crl_file="revoked-crl", expected=8)
    run(exe, "no-local-crl", crl_file=None, expected=7)
    run(exe, "truncated", behavior="truncated", expected=4)
    run(exe, "multi", behavior="multi", expected=10)
    run(exe, "oversize-body", behavior="oversize", expected=4)
    run(exe, "handshake-deadline", behavior="handshake-stall", expected=3, budget=200)
    run(exe, "read-deadline", behavior="read-stall", expected=3, budget=200)
    run(exe, "handshake-cancel", behavior="handshake-stall", expected=2, cancel=100)
    run(exe, "read-cancel", behavior="read-stall", expected=2, cancel=100)
  print("PASS", OUT)


if __name__ == "__main__":
    main()
