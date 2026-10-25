#!/usr/bin/env python3
"""Download verification-only samples and verify their pinned SHA-256 hashes."""
import concurrent.futures, hashlib, json, pathlib, re, subprocess, sys
manifest=pathlib.Path(__file__).resolve().parents[1]/'reports/raw-heif-samples.json'
destination=pathlib.Path(sys.argv[1]);destination.mkdir(parents=True,exist_ok=True)
def fetch(item):
    name=re.sub(r'[<>:"/\\|?*]', '_', item['name'])
    path=destination/name
    if not path.exists() or hashlib.sha256(path.read_bytes()).hexdigest()!=item['sha256']:
        subprocess.run(['curl','--fail','--location','--silent','--show-error','--max-time','180',item['url'],'--output',str(path)],check=True)
    if hashlib.sha256(path.read_bytes()).hexdigest()!=item['sha256']:raise RuntimeError(f'Checksum mismatch: {name}')
    return name
with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    for name in pool.map(fetch,json.loads(manifest.read_text())):print(name)
