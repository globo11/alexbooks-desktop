"""Makes the certificate authorities that src-tauri/src/trust.rs is tested against (src-tauri/testdata/*.pem).
Run once with Python and the `cryptography` package; the files are kept in the repository."""
import datetime as dt
import ipaddress
import sys
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

out = Path(sys.argv[1] if len(sys.argv) > 1 else "src-tauri/testdata"); out.mkdir(parents=True, exist_ok=True)


def make(name: str, permitted=None, critical=True):
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, f"Test authority ({name})")])
    now = dt.datetime(2026, 1, 1, tzinfo=dt.timezone.utc)
    b = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject).public_key(key.public_key()).serial_number(x509.random_serial_number())
         .not_valid_before(now).not_valid_after(now + dt.timedelta(days=3650))
         .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True))
    if permitted is not None:
        b = b.add_extension(x509.NameConstraints(permitted_subtrees=permitted, excluded_subtrees=None), critical=critical)
    (out / f"ca-{name}.pem").write_bytes(b.sign(key, hashes.SHA256()).public_bytes(serialization.Encoding.PEM))


make("office", [x509.DNSName("localhost"), x509.DNSName("SERVER01"), x509.DNSName("frontdesk.local")])
make("open")
make("dotcom", [x509.DNSName("localhost"), x509.DNSName("com")])
make("one-website", [x509.DNSName("www.mybank.example")])
make("soft", [x509.DNSName("localhost"), x509.DNSName("SERVER01")], critical=False)
make("numbers-only", [x509.IPAddress(ipaddress.ip_network("192.168.0.0/16"))])
