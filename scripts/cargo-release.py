#!/usr/bin/env python3
"""Run Cargo with reproducible, anonymous source paths on every desktop OS."""
import os
from pathlib import Path
import shlex
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
env = os.environ.copy()
flags = env.get('CARGO_ENCODED_RUSTFLAGS', '').split('\x1f') if env.get('CARGO_ENCODED_RUSTFLAGS') else shlex.split(env.get('RUSTFLAGS', ''))
# Later mappings win: preserve a useful project-relative path within the home mapping.
for source, target in [(Path.home(), '/build/home'),
                       (Path(env.get('CARGO_HOME', Path.home() / '.cargo')), '/build/cargo'),
                       (Path(env.get('RUSTUP_HOME', Path.home() / '.rustup')), '/build/rustup'),
                       (root, '/source/AdvancedExplorer')]:
    flags.append(f'--remap-path-prefix={source.resolve()}={target}')
env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(flags)
env.pop('RUSTFLAGS', None)
command = ['cargo'] if os.name == 'nt' else ['sh', str(root / 'scripts/cargo-local.sh')]
raise SystemExit(subprocess.call(command + sys.argv[1:], env=env))
