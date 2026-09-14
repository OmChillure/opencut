#!/usr/bin/env python3
"""Transcribe a wav with local faster-whisper and print OpenAI-style JSON."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path


def model_dir() -> str:
    env = os.environ.get("WHISPER_MODEL_DIR", "").strip()
    if env and Path(env).is_dir():
        return env
    here = Path(__file__).resolve().parents[1]
    local = here / "data" / "whisper" / "faster-whisper-base"
    if local.is_dir() and (local / "model.bin").is_file():
        return str(local)
    return os.environ.get("WHISPER_MODEL", "base")


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: faster_whisper_transcribe.py <audio>", file=sys.stderr)
        return 2
    wav = sys.argv[1]
    from faster_whisper import WhisperModel

    name = model_dir()
    model = WhisperModel(name, device="cpu", compute_type="int8")
    segments, info = model.transcribe(wav, beam_size=5, vad_filter=True)
    segs = []
    parts = []
    for s in segments:
        text = s.text.strip()
        if not text:
            continue
        segs.append({"start": s.start, "end": s.end, "text": text})
        parts.append(text)
    out = {
        "text": " ".join(parts),
        "language": info.language,
        "segments": segs,
    }
    json.dump(out, sys.stdout)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
