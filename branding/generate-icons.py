#!/usr/bin/env python3
"""Иконки приложения и марка в интерфейсе — из исходников бренда в этом каталоге.

Запуск из корня репозитория:

    python3 branding/generate-icons.py

Нужны `rsvg-convert` (librsvg) и Pillow. Скрипт перезаписывает `crates/vmixrtc/icons/*`
(десктоп, Windows, macOS, Android, iOS) и `crates/vmixrtc/ui/brand/*`, поэтому `tauri-cli`
для смены иконки не нужен.

Правила из брендбука, которые здесь зашиты:

* форма R и сетка 4×5 не меняются — все варианты получаются из одного SVG;
* ниже 24 px идёт моно-версия на Rust Orange (16 px в `.ico`, 20 px в iOS);
* охранное поле: у macOS иконка занимает 824/1024 холста, у Android — безопасную зону.
"""

from __future__ import annotations

import io
import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image

BRAND = Path(__file__).resolve().parent
ROOT = BRAND.parent
ICONS = ROOT / "crates" / "vmixrtc" / "icons"
UI_BRAND = ROOT / "crates" / "vmixrtc" / "ui" / "brand"

PANEL = "#1E2327"
RUST = "#F26B21"
PLATE = f'<rect width="512" height="512" rx="112" fill="{PANEL}"/>'
HEAD = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="{box}">'


def cells(svg: str) -> str:
    """Клетки сетки без обёртки `<svg>`, заголовка и подложки."""
    body = svg[svg.index("</title>") + len("</title>") : svg.rindex("</svg>")]
    return body.replace(PLATE, "")


def variants() -> dict[str, str]:
    colour = (BRAND / "vmixrtc-icon.svg").read_text(encoding="utf-8")
    mono = (BRAND / "vmixrtc-icon-mono.svg").read_text(encoding="utf-8")
    if PLATE not in colour:
        sys.exit("vmixrtc-icon.svg: не найдена подложка — изменилась разметка исходника")

    grid = cells(colour)
    mono_grid = cells(mono)
    square = f'<rect width="512" height="512" fill="{PANEL}"/>'
    return {
        # основная: скруглённая плашка во весь холст
        "plate": colour,
        # macOS: плашка занимает 824/1024 холста, вокруг прозрачное поле
        "mac": HEAD.format(box="-62 -62 636 636") + PLATE + grid + "</svg>",
        # iOS: непрозрачный квадрат, скругление делает система
        "square": HEAD.format(box="0 0 512 512") + square + grid + "</svg>",
        # Android adaptive: только клетки, плашка — это цвет фона слоя (72 из 108 dp видимы)
        "foreground": HEAD.format(box="-128 -128 768 768") + grid + "</svg>",
        # Android round: круг, сетка ужата, чтобы углы не обрезались
        "round": HEAD.format(box="0 0 512 512")
        + f'<circle cx="256" cy="256" r="256" fill="{PANEL}"/>'
        + f'<g transform="translate(256 256) scale(0.8) translate(-256 -256)">{grid}</g></svg>',
        # меньше 24 px: моно на цвете
        "tiny": HEAD.format(box="0 0 512 512")
        + f'<rect width="512" height="512" rx="112" fill="{RUST}"/>{mono_grid}</svg>',
        "tiny-square": HEAD.format(box="0 0 512 512")
        + f'<rect width="512" height="512" fill="{RUST}"/>{mono_grid}</svg>',
    }


def render(svg: str, size: int) -> Image.Image:
    png = subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size)],
        input=svg.encode("utf-8"),
        capture_output=True,
        check=True,
    ).stdout
    return Image.open(io.BytesIO(png)).convert("RGBA")


def save(image: Image.Image, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path, optimize=True)


def main() -> None:
    if not shutil.which("rsvg-convert"):
        sys.exit("нужен rsvg-convert (пакет librsvg)")
    svg = variants()

    def plate(size: int) -> Image.Image:
        return render(svg["tiny"] if size < 24 else svg["plate"], size)

    def opaque(size: int) -> Image.Image:
        return render(svg["tiny-square"] if size < 24 else svg["square"], size).convert("RGB")

    # десктоп (Linux-пакеты, bundle.icon) и исходник для `cargo tauri icon`
    for name, size in {
        "32x32.png": 32,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        "source.png": 1024,
    }.items():
        save(plate(size), ICONS / name)

    # Windows Store / MSIX
    for size in (30, 44, 71, 89, 107, 142, 150, 284, 310):
        save(plate(size), ICONS / f"Square{size}x{size}Logo.png")
    save(plate(50), ICONS / "StoreLogo.png")

    # Windows .ico
    ico_sizes = [16, 24, 32, 48, 64, 256]
    plate(256).save(
        ICONS / "icon.ico",
        format="ICO",
        sizes=[(size, size) for size in ico_sizes],
        append_images=[plate(size) for size in ico_sizes[:-1]],
    )

    # macOS .icns
    mac_sizes = [16, 32, 64, 128, 256, 512, 1024]
    render(svg["mac"], 1024).save(
        ICONS / "icon.icns",
        format="ICNS",
        append_images=[render(svg["mac"], size) for size in mac_sizes[:-1]],
    )

    # Android
    for density, scale in {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}.items():
        folder = ICONS / "android" / f"mipmap-{density}"
        save(plate(round(48 * scale)), folder / "ic_launcher.png")
        save(render(svg["round"], round(48 * scale)), folder / "ic_launcher_round.png")
        save(render(svg["foreground"], round(108 * scale)), folder / "ic_launcher_foreground.png")
    (ICONS / "android" / "values" / "ic_launcher_background.xml").write_text(
        '<?xml version="1.0" encoding="utf-8"?>\n<resources>\n'
        f'  <color name="ic_launcher_background">{PANEL}</color>\n</resources>\n',
        encoding="utf-8",
    )

    # iOS
    for name, size in {
        "20x20@1x": 20, "20x20@2x": 40, "20x20@2x-1": 40, "20x20@3x": 60,
        "29x29@1x": 29, "29x29@2x": 58, "29x29@2x-1": 58, "29x29@3x": 87,
        "40x40@1x": 40, "40x40@2x": 80, "40x40@2x-1": 80, "40x40@3x": 120,
        "60x60@2x": 120, "60x60@3x": 180,
        "76x76@1x": 76, "76x76@2x": 152,
        "83.5x83.5@2x": 167,
        "512@2x": 1024,
    }.items():
        save(opaque(size), ICONS / "ios" / f"AppIcon-{name}.png")

    # марка в интерфейсе: значок (он же favicon) и вордмарк, вырезанный из основного логотипа
    UI_BRAND.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(BRAND / "vmixrtc-icon.svg", UI_BRAND / "icon.svg")
    logo = (BRAND / "vmixrtc-logo.svg").read_text(encoding="utf-8")
    start = logo.index('<g transform="translate(560,330)">')
    wordmark = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="561 214 646 120" role="img" '
        'aria-label="vMixRTC"><title>vMixRTC</title>' + logo[start : logo.rindex("</svg>")] + "</svg>\n"
    )
    (UI_BRAND / "wordmark.svg").write_text(wordmark, encoding="utf-8")

    print(f"иконки: {ICONS.relative_to(ROOT)}, марка: {UI_BRAND.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
