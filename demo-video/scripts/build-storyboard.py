#!/usr/bin/env python3
"""Assemble the review poses and the HTML board from the approved plan files."""
import argparse
import html
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent


def frames():
    text = (ROOT / "STORYBOARD.md").read_text()
    matches = re.finditer(r"^## Frame (\d+) — (.+?) \([^\n]+\)\n(.*?)(?=^## Frame |\Z)", text, re.M | re.S)
    result = []
    start = 0.0
    for match in matches:
        fields = dict(re.findall(r"^- ([a-z_]+): (.*)$", match[3], re.M))
        duration = float(fields["duration"].removesuffix("s"))
        result.append({**fields, "number": int(match[1]), "title": match[2],
                       "start": start, "duration": duration,
                       "mid": start + float(fields["poster"].removesuffix("s")),
                       "id": Path(fields["src"]).stem.split("-")[0]})
        start += duration
    assert len(result) == 16 and abs(start - 120) < 0.001
    return result


def poses(shots):
    data = json.loads((ROOT / "assets/demo-data.json").read_text())
    audio_path = ROOT / "assets/voice/audio_meta.json"
    audio = json.loads(audio_path.read_text()) if audio_path.exists() else None
    variables = [
        {"id": "color-ground", "type": "color", "label": "Background", "default": "#000000"},
        {"id": "color-ink", "type": "color", "label": "Text", "default": "#FFFFFF"},
        {"id": "color-accent", "type": "color", "label": "Accent", "default": "#5CF26A"},
        {"id": "model-name", "type": "string", "label": "Model", "default": data["model"]["name"]},
        {"id": "model-layers", "type": "number", "label": "Example total layers", "default": data["model"]["layers"]},
        {"id": "laptop-layers", "type": "number", "label": "Example laptop layers", "default": data["allocation"]["laptopLayers"]},
        {"id": "phone-layers", "type": "number", "label": "Example phone layers", "default": data["allocation"]["phoneLayers"]},
    ]
    for shot in shots:
        variables.append({"id": f'{shot["id"]}-title', "type": "string", "label": f'{shot["number"]:02d} title', "default": shot["title"]})
    hosts = []
    captions = []
    for shot in shots:
        hosts.append(f'''    <div id="{shot['id']}" class="clip" data-composition-id="{shot['id']}" data-composition-src="{shot['src']}" data-start="{shot['start']:g}" data-duration="{shot['duration']:g}" data-track-index="1" data-track-kind="graphics" data-width="1920" data-height="1080"></div>''')
        clip = audio["clips"][shot["number"] - 1] if audio else None
        caption_start = clip["start"] if clip else shot["start"]
        caption_duration = clip["duration"] if clip else shot["duration"]
        captions.append(f'''    <div id="caption-{shot['number']:02d}" class="clip" data-start="{caption_start:g}" data-duration="{caption_duration:g}" data-track-index="0"><div class="caption">{html.escape(shot['voiceover'])}</div></div>''')
    audio_tags = []
    if audio:
        for clip in audio["clips"]:
            audio_tags.append(f'''    <audio id="voice-{clip['number']:02d}" class="clip" src="{clip['path']}" data-start="{clip['start']:g}" data-duration="{clip['duration']:g}" data-track-index="3" data-volume="1"></audio>''')
    root = f'''<!doctype html>
<html lang="en" data-resolution="landscape" data-composition-variables='{html.escape(json.dumps(variables), quote=True)}'>
<head><meta charset="utf-8"><meta name="viewport" content="width=1920,height=1080">
<link rel="stylesheet" href="assets/video.css">
<script src="assets/vendor/gsap.min.js"></script>
<style>html,body{{margin:0;width:1920px;height:1080px;overflow:hidden;background:#000}}#root{{position:absolute;inset:0}}.clip{{position:absolute;inset:0}}</style></head>
<body><div id="root" data-composition-id="main" data-start="0" data-duration="120" data-width="1920" data-height="1080" data-fps="30" data-no-timeline>
{chr(10).join(hosts)}
    <div id="captions" class="clip" data-composition-id="captions" data-composition-src="compositions/captions.html" data-start="0" data-duration="120" data-track-index="2" data-track-kind="captions" data-width="1920" data-height="1080" data-no-timeline style="z-index:100"></div>
{chr(10).join(audio_tags)}
</div></body></html>
'''
    (ROOT / "index.html").write_text(root)
    (ROOT / "compositions/captions.html").write_text(f'''<template>
<link rel="stylesheet" href="assets/video.css">
<style>#root{{position:absolute;inset:0;pointer-events:none}}.clip{{position:absolute;inset:0}}</style>
<div id="root" data-composition-id="captions" data-width="1920" data-height="1080" data-duration="120" data-no-timeline>
{chr(10).join(captions)}
</div></template>
''')
    (ROOT / "boards/shots.json").write_text(json.dumps(shots, indent=2))
    print("Pose timestamps:", ",".join(f'{shot["mid"]:g}' for shot in shots))


def qr():
    import qrcode
    import qrcode.image.svg
    code = qrcode.QRCode(version=None, error_correction=qrcode.constants.ERROR_CORRECT_M, box_size=10, border=4)
    code.add_data("localhost.ai storyboard demo — this is not a pairing code")
    code.make(fit=True)
    image = code.make_image(image_factory=qrcode.image.svg.SvgPathImage)
    image.save(str(ROOT / "assets/brand/demo-qr.svg"))


def board(shots):
    cards = []
    last_act = None
    for index, shot in enumerate(shots):
        if shot["act"] != last_act:
            title = "Part 1 · How it works" if shot["act"] == "Explain" else "Part 2 · Recreated UI demo"
            cards.append(f'<section class="act">{title}</section>')
            last_act = shot["act"]
        source = f"stills/frame-{index:02d}-at-{shot['mid']:g}s.png"
        if not (ROOT / "boards" / source).exists():
            raise FileNotFoundError(source)
        cards.append(f'''<div class="frame" data-frame="{shot['number']}">
<img src="{source}" alt="Frame {shot['number']}: {html.escape(shot['title'], quote=True)}">
<div class="copy"><div class="title">{shot['number']:02d}: {html.escape(shot['title'])}</div>
<div class="time">{shot['start']:.1f} to {shot['start'] + shot['duration']:.1f}s</div>
<p class="words"><strong>Action</strong> {html.escape(shot['scene'])}
<span class="narration"><strong>Narration</strong> {html.escape(shot['voiceover'])}</span></p></div></div>''')
    (ROOT / "boards/storyboard.html").write_text(f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Make room — localhost.ai storyboard</title>
<style>*{{box-sizing:border-box}}body{{margin:0;background:#f5f5f4;color:#171719;font-family:Inter,Arial,sans-serif}}header{{max-width:1640px;margin:auto;padding:48px 32px 18px}}h1{{font-size:42px;letter-spacing:-1.4px;margin:0 0 12px}}header p{{font-size:17px;line-height:1.6;max-width:950px;color:#515158}}.grid{{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:28px;max-width:1640px;margin:auto;padding:0 32px 64px}}.act{{grid-column:1/-1;font-size:25px;font-weight:700;padding:28px 0 0}}.frame{{background:#fff;border:1px solid #dddddf;border-radius:18px;overflow:hidden}}.frame img{{display:block;width:100%;aspect-ratio:16/9;object-fit:contain;background:#000}}.copy{{padding:22px 24px 24px}}.title{{font-size:21px;font-weight:700;line-height:1.3}}.time{{font-size:14px;color:#66666e;margin:8px 0 16px;font-variant-numeric:tabular-nums}}.words{{font-size:15px;line-height:1.55;margin:8px 0;color:#494951}}.words strong{{color:#171719;margin-right:6px}}.narration{{display:block;margin-top:10px;padding-top:10px;border-top:1px solid #ededee}}@media(max-width:900px){{.grid{{grid-template-columns:1fr}}header h1{{font-size:34px}}}}</style></head>
<body><header><h1>Make room</h1><p><strong>localhost.ai · 2 minutes · 1920×1080</strong><br>
Review the explanation, recreated UI, and narration below. These are still poses for approval; movement and recorded voice follow the review. The allocation is an illustrative 25+3 example.</p></header>
<main class="grid">{chr(10).join(cards)}</main></body></html>
''')
    from PIL import Image, ImageDraw, ImageFont
    sheet = Image.new("RGB", (1940, 1220), "#19191c")
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.truetype(str(ROOT / "assets/fonts/Outfit-Medium.ttf"), 18)
    for index, shot in enumerate(shots):
        x, y = 12 + (index % 4) * 484, 10 + (index // 4) * 304
        draw.text((x + 2, y), f"{shot['number']:02d}  {shot['start']:g}–{shot['start'] + shot['duration']:g}s  {shot['title']}", font=font, fill="white")
        path = ROOT / "boards/stills" / f"frame-{index:02d}-at-{shot['mid']:g}s.png"
        with Image.open(path) as still:
            sheet.paste(still.convert("RGB").resize((472, 266)), (x, y + 28))
    sheet.save(ROOT / "boards/contact-sheet.jpg", quality=95)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--poses", action="store_true")
    parser.add_argument("--board", action="store_true")
    parser.add_argument("--restore-film", action="store_true")
    options = parser.parse_args()
    shots = frames()
    if options.poses:
        qr()
        poses(shots)
    if options.board:
        board(shots)
    if options.restore_film:
        (ROOT / "boards/.posed.html").write_text((ROOT / "index.html").read_text())
        (ROOT / "index.html").write_text((ROOT / "boards/.film.html").read_text())


if __name__ == "__main__":
    main()
