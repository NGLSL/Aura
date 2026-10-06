"""Generate detached, synthetic CTL fixtures without installing keys or trust.

Run with an existing cryptography-enabled Python. Private keys stay in memory;
only public signer certificates and signed CTLs are written beside this script.
These fixtures are NOT Microsoft AuthRoot material.
"""
from pathlib import Path
from datetime import datetime, timezone
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa, padding
from cryptography.x509.oid import NameOID, ObjectIdentifier

OUT = Path(__file__).parent / "fixtures"
OUT.mkdir(exist_ok=True)

def der(tag, data):
    size = len(data)
    length = bytes([size]) if size < 128 else bytes([128 + (size.bit_length()+7)//8]) + size.to_bytes((size.bit_length()+7)//8, 'big')
    return bytes([tag]) + length + data

def seq(*items): return der(0x30, b''.join(items))
def integer(n):
    b = n.to_bytes(max(1, (n.bit_length()+7)//8), 'big')
    return der(2, (b'\0' if b[0] & 128 else b'') + b)
def oid(s):
    values = list(map(int, s.split('.')))
    out = bytes([40*values[0]+values[1]])
    for n in values[2:]:
        b = [n & 127]
        while n >> 7:
            n >>= 7
            b.insert(0, (n & 127) | 128)
        out += bytes(b)
    return der(6, out)
def utc(s): return der(0x17, s.encode('ascii'))
def alg(s): return seq(oid(s), der(5, b''))
SHA256 = '2.16.840.1.101.3.4.2.1'
RSA = '1.2.840.113549.1.1.1'
USAGE = '1.3.6.1.4.1.311.10.3.9'

def signer(serial, eku=USAGE):
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, f'Aura synthetic CTL signer {serial}')])
    cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name).public_key(key.public_key()).serial_number(serial)
            .not_valid_before(datetime(2025,1,1,tzinfo=timezone.utc)).not_valid_after(datetime(2035,1,1,tzinfo=timezone.utc))
            .add_extension(x509.ExtendedKeyUsage([ObjectIdentifier(eku)]), critical=True).sign(key, hashes.SHA256()))
    return key, cert

A = signer(101)
B = signer(102)
C = signer(103, '1.3.6.1.5.5.7.3.1')
for name, (_, cert) in [('signer',A), ('wrong-signer',B), ('wrong-eku',C)]:
    (OUT/f'{name}.der').write_bytes(cert.public_bytes(serialization.Encoding.DER))

def ctl(sequence=7, start='261001000000Z', end='261101000000Z', unknown=False, list_id=b'Aura research v1', subject_alg=SHA256):
    entry = seq(der(4, bytes(range(32))), der(0x31, seq(oid('1.2.3.4'), der(0x31, der(4,b'unknown'))))) if unknown else seq(der(4, bytes(range(32))))
    return seq(integer(0), seq(oid(USAGE)), der(4,list_id), integer(sequence), utc(start), utc(end) if end else b'', alg(subject_alg), seq(entry))

def signed(content, signers, weak=False):
    infos = []
    certs = []
    for key, cert in signers:
        certs.append(cert.public_bytes(serialization.Encoding.DER))
        # PKCS#7 signs the value octets of a non-Data content object; its DER
        # tag/length are not part of the signed digest (RFC 2315 section 9.3).
        header = 2 if content[1] < 128 else 2 + (content[1] & 127)
        digest = '1.3.14.3.2.26' if weak else SHA256
        signature = key.sign(content[header:], padding.PKCS1v15(), hashes.SHA1() if weak else hashes.SHA256())
        infos.append(seq(integer(1), seq(cert.issuer.public_bytes(), integer(cert.serial_number)), alg(digest), alg(RSA), der(4,signature)))
    data = seq(integer(1), der(0x31,alg('1.3.14.3.2.26' if weak else SHA256)), seq(oid('1.3.6.1.4.1.311.10.1'),der(0xa0,content)), der(0xa0,b''.join(certs)), der(0x31,b''.join(infos)))
    return seq(oid('1.2.840.113549.1.7.2'),der(0xa0,data))

cases = {'valid':(ctl(),[A]), 'unsigned':(ctl(),[]), 'expired':(ctl(end='261005000000Z'),[A]), 'future':(ctl(start='261007000000Z'),[A]),
         'rollback':(ctl(sequence=6),[A]), 'unknown-policy':(ctl(unknown=True),[A]), 'multiple-unknown':(ctl(),[A,B]),
         'multiple-known':(ctl(),[A,A]), 'wrong-eku':(ctl(),[C]), 'wrong-list':(ctl(list_id=b'other list'),[A]),
         'missing-next-update':(ctl(end=None),[A]), 'unknown-algorithm':(ctl(subject_alg='1.2.3.4'),[A])}
for name,(content,keys) in cases.items(): (OUT/f'{name}.stl').write_bytes(signed(content,keys))
(OUT/'weak-signature.stl').write_bytes(signed(ctl(), [A], weak=True))
tampered = bytearray((OUT/'valid.stl').read_bytes())
where = tampered.index(bytes(range(32)))
tampered[where] ^= 1
(OUT/'tampered.stl').write_bytes(tampered)
