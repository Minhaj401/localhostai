#!/usr/bin/env python3
"""Generate the approved narration as one measured Kokoro clip per scene."""
import json
import os
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
PYTHON = os.environ.get("HYPERFRAMES_PYTHON", "/tmp/opencode/kokoro-venv/bin/python")
VOICE = "am_adam"
SPEED = "1.08"


def script_lines():
    text = (ROOT / "SCRIPT.md").read_text()
    matches = re.finditer(r"^## (\d+) — (.+?) \(([^)]+)\)\n([^\n]+)", text, re.M)
    result = []
    for match in matches:
        start, end = [float(part.removesuffix("s")) for part in match[3].split("–")]
        result.append({"number": int(match[1]), "title": match[2], "start": start,
                       "slotEnd": end, "text": match[4].strip()})
    assert len(result) == 16
    return result


def duration(path):
    output = subprocess.check_output([
        "ffprobe", "-v", "error", "-show_entries", "format=duration",
        "-of", "default=noprint_wrappers=1:nokey=1", str(path)
    ], text=True)
    return float(output.strip())


def main():
    out = ROOT / "assets/voice"
    out.mkdir(parents=True, exist_ok=True)
    records = []
    for line in script_lines():
        text_path = out / f"scene-{line['number']:02d}.txt"
        wav_path = out / f"scene-{line['number']:02d}.wav"
        text_path.write_text(line["text"] + "\n")
        subprocess.run([
            "npx", "--yes", "hyperframes@0.9.26", "tts", str(text_path),
            "--voice", VOICE, "--speed", SPEED, "-o", str(wav_path), "--json"
        ], cwd=ROOT, env={**os.environ, "HYPERFRAMES_PYTHON": PYTHON}, check=True)
        seconds = duration(wav_path)
        available = line["slotEnd"] - line["start"] - 0.7
        if seconds > available:
            raise RuntimeError(f"Scene {line['number']:02d} narration is {seconds:.2f}s; slot allows {available:.2f}s")
        records.append({**line, "path": f"assets/voice/{wav_path.name}",
                        "duration": round(seconds, 3), "start": line["start"] + 0.35,
                        "voice": VOICE, "speed": float(SPEED)})
    (ROOT / "assets/voice/audio_meta.json").write_text(json.dumps({
        "voice_credit": "Adam voice generated locally with Kokoro-82M",
        "provider": "kokoro-local",
        "voice": VOICE,
        "speed": float(SPEED),
        "clips": records,
    }, indent=2))
    print(json.dumps({"clips": len(records), "maxDuration": max(r["duration"] for r in records),
                      "totalSpeech": round(sum(r["duration"] for r in records), 3)}, indent=2))


if __name__ == "__main__":
    main()
