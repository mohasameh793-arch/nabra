"""Render the brand SVGs to PNG/ICO with headless Microsoft Edge (exact SVG rendering, Arabic shaping, transparency).

    python brand/export.py

Writes: desktop/icons/icon.ico (app + tray icon), brand/png/*.png, brand/social-preview.png (GitHub card).
"""
import struct
import subprocess
import tempfile
from pathlib import Path

HERE = Path(__file__).parent
EDGE = Path(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
ICON_SIZES = [16, 24, 32, 48, 64, 128, 256]


def shot(html: str, width: int, height: int, out: Path) -> bytes:
    """Screenshot `html` at width×height with a transparent background."""
    with tempfile.TemporaryDirectory() as tmp:
        page = Path(tmp) / "page.html"
        page.write_text(html, encoding="utf-8")
        subprocess.run([str(EDGE), "--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
                        "--default-background-color=00000000", f"--window-size={width},{height}",
                        f"--screenshot={out}", page.as_uri()], check=True, capture_output=True, timeout=60)
    return out.read_bytes()


def page(svg: Path, width: int, height: int, background: str = "transparent", extra: str = "") -> str:
    return (f"<!doctype html><meta charset='utf-8'><style>html,body{{margin:0;background:{background};width:{width}px;"
            f"height:{height}px;overflow:hidden}}img{{display:block;width:{width}px;height:{height}px}}</style>"
            f"<img src='{svg.as_uri()}'>{extra}")


def ico(pngs: list[tuple[int, bytes]]) -> bytes:
    """ICO container with embedded PNGs (supported since Windows Vista)."""
    offset = 6 + 16 * len(pngs)
    head, body = struct.pack("<HHH", 0, 1, len(pngs)), b""
    for size, data in pngs:
        head += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
        body += data
    return head + body


def main() -> None:
    out = HERE / "png"
    out.mkdir(exist_ok=True)
    icon = HERE / "nabra-app-icon.svg"
    pngs = [(s, shot(page(icon, s, s), s, s, out / f"app-icon-{s}.png")) for s in ICON_SIZES]
    target = HERE.parent / "desktop" / "icons" / "icon.ico"
    target.write_bytes(ico([p for p in pngs if p[0] in (16, 24, 32, 48, 256)]))
    shot(page(HERE / "nabra-logo.svg", 840, 192), 840, 192, out / "logo-light.png")
    shot(page(HERE / "nabra-logo-dark.svg", 840, 192), 840, 192, out / "logo-dark.png")

    # GitHub social preview (Settings → Social preview): 1280×640.
    card = f"""<!doctype html><meta charset='utf-8'><style>
      html,body{{margin:0;width:1280px;height:640px;background:#141413;overflow:hidden;font-family:'Segoe UI',sans-serif}}
      .wrap{{position:absolute;inset:0;display:flex;flex-direction:column;justify-content:center;padding:0 110px;gap:30px;
        background:radial-gradient(60% 80% at 92% 20%,rgba(43,179,163,.28),transparent 60%),radial-gradient(50% 70% at 80% 110%,rgba(245,168,61,.18),transparent 60%)}}
      img{{width:840px;height:192px;margin-left:-6px}}
      p{{margin:0;color:#d9d6cf;font-size:34px;line-height:1.35;max-width:980px}}
      small{{color:#9b968d;font-size:24px}}
    </style><div class='wrap'><img src='{(HERE / "nabra-logo-dark.svg").as_uri()}'>
      <p>Free, open-source voice typing and meeting notes that run on your own PC. Arabic, English, and both at once.</p>
      <small>Local-first · Offline · No account · MIT</small></div>"""
    shot(card, 1280, 640, HERE / "social-preview.png")
    print("wrote", target, "and", out, "and social-preview.png")


if __name__ == "__main__":
    main()
