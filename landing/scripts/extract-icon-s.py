#!/usr/bin/env python3
"""앱 아이콘 PNG에서 S 리본만 분리해 랜딩용 투명 배경 마크(mark.png)를 만든다.

흰 배경과의 거리로 내부를 구하고, 가장자리 띠는 가장 가까운 내부 픽셀의
색을 가져와 알파만 계산한다. 색을 다시 그리지 않으므로 아이콘의 형태와
음영이 그대로 남는다.

    python3 landing/scripts/extract-icon-s.py app/assets/StologyLocal.png landing/public

필요 패키지: numpy, pillow, scipy
"""
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

CORE_DISTANCE = 0.16   # 흰색과 이만큼 떨어지면 확실한 내부
EDGE_REACH = 4         # 내부 바깥으로 알파를 계산할 폭(px)
PADDING = 0.035        # 잘라낸 뒤 남기는 여백 비율


def extract(source: Path) -> Image.Image:
    rgb = np.asarray(Image.open(source).convert('RGB'), dtype=np.float32) / 255
    distance = (1 - rgb).max(axis=2)
    core = ndimage.binary_erosion(distance > CORE_DISTANCE, iterations=2)
    # 가장 큰 덩어리만 남겨 배경의 미세한 얼룩을 버린다.
    labels, count = ndimage.label(core)
    if count > 1:
        sizes = ndimage.sum(core, labels, range(1, count + 1))
        core = labels == (int(np.argmax(sizes)) + 1)

    gap, nearest = ndimage.distance_transform_edt(~core, return_indices=True)
    near_rgb = rgb[nearest[0], nearest[1]]
    near_distance = distance[nearest[0], nearest[1]]
    alpha = np.clip(distance / np.maximum(near_distance, 1e-3), 0, 1)
    alpha[gap > EDGE_REACH + 2] = 0
    alpha[core] = 1
    out = np.dstack([np.where(core[..., None], rgb, near_rgb), alpha])

    ys, xs = np.nonzero(alpha > 0.01)
    size = max(ys.max() - ys.min(), xs.max() - xs.min()) + 1
    pad = int(round(size * PADDING))
    top, left = max(ys.min() - pad, 0), max(xs.min() - pad, 0)
    bottom, right = ys.max() + pad + 1, xs.max() + pad + 1
    cropped = out[top:bottom, left:right]
    print(f'S 영역: x {xs.min()}–{xs.max()}, y {ys.min()}–{ys.max()} → {cropped.shape[1]}×{cropped.shape[0]}')
    return Image.fromarray((cropped * 255 + .5).astype(np.uint8), 'RGBA')


def square(image: Image.Image, size: int) -> Image.Image:
    side = max(image.size)
    canvas = Image.new('RGBA', (side, side), (0, 0, 0, 0))
    canvas.paste(image, ((side - image.width) // 2, (side - image.height) // 2))
    return canvas.resize((size, size), Image.LANCZOS)


def main() -> None:
    source, public = Path(sys.argv[1]), Path(sys.argv[2])
    ribbon = extract(source)
    square(ribbon, 192).save(public / 'mark.png', optimize=True)
    print('저장:', public / 'mark.png')


if __name__ == '__main__':
    main()
