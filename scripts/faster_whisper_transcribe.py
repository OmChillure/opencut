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


def transcribe_one(model, wav: str) -> dict:
    # beam_size=1 is ~3–5× faster on CPU than 5 and good enough for cuts.
    segments, info = model.transcribe(
        wav, beam_size=1, vad_filter=True, condition_on_previous_text=False
    )
    segs = []
    parts = []
    for s in segments:
        text = s.text.strip()
        if not text:
            continue
        segs.append({"start": s.start, "end": s.end, "text": text})
        parts.append(text)
        print(f"  {s.start:.1f}-{s.end:.1f}s {text[:80]}", file=sys.stderr, flush=True)
    return {
        "text": " ".join(parts),
        "language": info.language,
        "segments": segs,
    }


def load_model():
    from faster_whisper import WhisperModel

    name = model_dir()
    print(f"loading whisper {name} (cpu int8)…", file=sys.stderr, flush=True)
    model = WhisperModel(name, device="cpu", compute_type="int8")
    print("whisper ready", file=sys.stderr, flush=True)
    return model


def serve() -> int:
    model = load_model()
    print("READY", file=sys.stderr, flush=True)
    for line in sys.stdin:
        wav = line.strip()
        if not wav or wav == "quit":
            break
        print(f"transcribe {wav}", file=sys.stderr, flush=True)
        try:
            out = transcribe_one(model, wav)
            print(json.dumps(out), flush=True)
        except Exception as exc:  # noqa: BLE001 — surface to the worker
            print(json.dumps({"error": str(exc)}), flush=True)
    return 0


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: faster_whisper_transcribe.py <audio>|--serve", file=sys.stderr)
        return 2
    if sys.argv[1] == "--serve":
        return serve()
    model = load_model()
    json.dump(transcribe_one(model, sys.argv[1]), sys.stdout)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
