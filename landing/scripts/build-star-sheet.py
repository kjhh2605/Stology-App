#!/usr/bin/env python3
"""render-star-sprites.py가 만든 프레임을 랜딩용 스프라이트 시트로 묶는다.

    python3 landing/scripts/build-star-sheet.py <프레임 폴더> landing/public/media/hero

시트는 8열, 칸 256px이다. 0–47번은 눈 뜬 한 바퀴, 48–62번은 눈 감은 정면 ±52.5°.
hero-scene.js의 SHEET 상수와 같은 값을 쓴다. 필요 패키지: pillow
"""
import sys
from pathlib import Path

from PIL import Image

CELL = 256
COLUMNS = 8
# 별의 중심(256, 240)에 맞춘 정사각 영역. 여백을 줄여 칸을 채운다.
CROP = (32, 16, 480, 464)
STILL_FRAME = 'open-03.png'


def cell(path: Path) -> Image.Image:
    # 곱한 알파로 줄여야 가장자리에 어두운 테두리가 생기지 않는다.
    frame = Image.open(path).convert('RGBA').crop(CROP).convert('RGBa')
    return frame.resize((CELL, CELL), Image.LANCZOS).convert('RGBA')


def main() -> None:
    frames, out = Path(sys.argv[1]), Path(sys.argv[2])
    names = sorted(p.name for p in frames.glob('open-*.png')) + sorted(p.name for p in frames.glob('blink-*.png'))
    if len(names) != 63:
        raise SystemExit(f'프레임 63개가 필요하다. 현재 {len(names)}개')
    rows = -(-len(names) // COLUMNS)
    sheet = Image.new('RGBA', (CELL * COLUMNS, CELL * rows), (0, 0, 0, 0))
    for index, name in enumerate(names):
        sheet.paste(cell(frames / name), (index % COLUMNS * CELL, index // COLUMNS * CELL))
    out.mkdir(parents=True, exist_ok=True)
    sheet.save(out / 'star-sheet.webp', quality=88, alpha_quality=95, method=6)
    cell(frames / STILL_FRAME).save(out / 'star.webp', quality=90, alpha_quality=95, method=6)
    print('저장:', out / 'star-sheet.webp', sheet.size, '·', out / 'star.webp')


if __name__ == '__main__':
    main()
