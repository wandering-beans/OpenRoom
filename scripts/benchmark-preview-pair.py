"""Alternate two frontend_benchmark_server instances to compare resident workers."""
import argparse
import json
import statistics
import time
import urllib.parse
import urllib.request
from pathlib import Path


def preview(endpoint, settings):
    request = urllib.request.Request(
        endpoint,
        data=json.dumps({"settings": settings, "bound": 1600}).encode(),
        headers={"Content-Type": "text/plain"},
    )
    started = time.perf_counter()
    with urllib.request.urlopen(request, timeout=120) as response:
        frame = json.load(response)
    if "error" in frame:
        raise RuntimeError(frame["error"])
    return frame, (time.perf_counter() - started) * 1000


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before")
    parser.add_argument("after")
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    endpoints = {"before": args.before, "after": args.after}
    for endpoint in endpoints.values():
        if urllib.parse.urlparse(endpoint).hostname != "127.0.0.1":
            parser.error("Use local benchmark servers on 127.0.0.1")
    detail = dict(version=1, contrast=17, highlights=-28, shadows=19, vibrance=12,
                  sharpen=40, noise_luma=15, texture=15, clarity=10)
    results = []
    for name, base, field in [
        ("neutral_exposure", {"version": 1}, "exposure"),
        ("detail_exposure", detail, "exposure"),
        ("detail_contrast", detail, "contrast"),
        ("detail_saturation", detail, "saturation"),
    ]:
        samples = {stage: [] for stage in endpoints}
        for index in range(26):
            value = (index - 12) * 0.05 if field == "exposure" else index + 10
            settings = {**base, field: value}
            frames = {}
            order = ["before", "after"] if index % 2 == 0 else ["after", "before"]
            for stage in order:
                frame, elapsed = preview(endpoints[stage], settings)
                frames[stage] = frame
                if index >= 5:
                    samples[stage].append({"request_ms": elapsed, "render_ms": frame["info"]["elapsed_ms"]})
            assert frames["before"]["inline_image"] == frames["after"]["inline_image"], f"Changed PNG output: {name} {index}"
            assert frames["before"]["info"]["histogram"] == frames["after"]["info"]["histogram"], f"Changed histogram: {name} {index}"
        result = {"scenario": name, "samples": 21, "matching_png_frames": 26}
        for stage, timings in samples.items():
            result[stage] = {
                key: {"median_ms": statistics.median(row[key] for row in timings),
                      "p95_ms": sorted(row[key] for row in timings)[19]}
                for key in ["request_ms", "render_ms"]
            }
        results.append(result)
        print(json.dumps(result), flush=True)
    args.report.write_text(json.dumps({"bound": 1600, "order": "alternating", "results": results}, indent=2) + "\n")


if __name__ == "__main__":
    main()
