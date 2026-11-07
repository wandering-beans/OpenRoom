"""Compare uncompressed RGB pixels produced by two staged desktop Release builds."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
from PIL import Image

parser = argparse.ArgumentParser()
parser.add_argument("before", type=Path)
parser.add_argument("after", type=Path)
parser.add_argument("folders", type=Path, nargs="+")
parser.add_argument("--serial-srf", action="store_true",
                    help="limit only SRF rendering to one OpenMP thread in both builds")
args = parser.parse_args()
extensions = set("cr2 cr3 crw nef nrw raf srw sr2 srf arw orf rw2 rwl pef iiq fff 3fr dng".split())
inputs = sorted(p for folder in args.folders for p in folder.rglob("*") if p.suffix.lower()[1:] in extensions)
edited = {"exposure": 0.45, "crop": [0.1, 0.12, 0.7, 0.75], "quarter_turns": 1, "flip_h": True,
          "masks": [{"name":"test","enabled":True,"invert":False,"amount":0.8,"adjustments":{"exposure":0.7},
                     "shapes":[{"kind":"linear","operation":"add","start":[0.2,0.2],"end":[0.7,0.7],
                                "radius":0.2,"feather":0.3,"opacity":1.0,"points":[],"color":[1,0,0],"range":[0,1]}]}]}
failed = 0
with tempfile.TemporaryDirectory(prefix="openroom-pixel-comparison-") as temp:
    base = Path(temp)
    for source in inputs:
        cases = [(1600, {})]
        if source.name.lower().endswith(".nef"):
            cases += [(384, edited), (1600, edited)]
        for bound, recipe in cases:
            (base / "recipe.json").write_text(json.dumps(recipe), encoding="utf8")
            results = []
            for exe in (args.before, args.after):
                dest = base / "frame.png"
                env = os.environ.copy()
                if args.serial_srf and source.suffix.lower() == ".srf":
                    env["OMP_THREAD_LIMIT"] = "1"
                subprocess.run([str(exe.resolve()), "--develop-agent-preview", str(source.resolve()),
                                str(dest), str(base / "recipe.json"), str(bound)],
                               check=True, stdout=subprocess.DEVNULL, timeout=180, env=env)
                with Image.open(dest) as frame:
                    results.append((frame.size, hashlib.sha256(frame.convert("RGB").tobytes()).hexdigest()))
            same = results[0] == results[1]
            failed += not same
            print(json.dumps(dict(file=source.name, bound=bound, edited=bool(recipe),
                                  same=same, before=results[0], after=results[1])), flush=True)
if not inputs or failed:
    raise SystemExit(f"{failed} mismatches across {len(inputs)} inputs")
