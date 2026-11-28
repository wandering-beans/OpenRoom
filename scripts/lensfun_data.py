"""Read the pinned Lensfun snapshot without using the network."""
import hashlib
import io
from pathlib import Path
import re
import tarfile
import xml.etree.ElementTree as ET

DIRECTORY = Path(__file__).resolve().parents[1] / 'native/rtengine/lensfun'
ARCHIVE = DIRECTORY / 'version_1.tar.bz2'
PIN = DIRECTORY / 'sha256.txt'

def profiles(blob=None):
    if blob is None:
        blob = ARCHIVE.read_bytes()
        if hashlib.sha256(blob).hexdigest() != PIN.read_text().strip():
            raise ValueError('Lensfun snapshot checksum mismatch')
    result = {}
    with tarfile.open(fileobj=io.BytesIO(blob), mode='r:bz2') as archive:
        for member in archive:
            name = member.name
            if (not member.isfile() or not re.fullmatch(r'[\w.-]+\.xml', name, re.ASCII)
                    or member.size > 8_000_000 or name in result
                    or len(result) >= 256 or sum(map(len, result.values())) + member.size > 32_000_000):
                raise ValueError(f'Unexpected Lensfun archive member: {name}')
            data = archive.extractfile(member).read()
            root = ET.fromstring(data)
            if root.tag != 'lensdatabase' or root.get('version') != '1':
                raise ValueError(f'Unsupported Lensfun database in {name}')
            result[name] = data
    if not result:
        raise ValueError('Empty Lensfun database')
    return result
