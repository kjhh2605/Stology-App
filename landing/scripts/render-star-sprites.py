"""라벤더 별 Blender 원본을 랜딩 히어로용 회전 프레임으로 렌더한다.

원본의 조명·재질·정면 직교 카메라를 그대로 쓰고 별만 세로축으로 돌린다.
배경은 투명으로 바꾸며 원본 파일은 저장하지 않는다.

    blender -b <lavender-star.blend> --python landing/scripts/render-star-sprites.py -- <출력 폴더>

출력: open-00.png … open-47.png (7.5° 간격 한 바퀴), blink-00.png … blink-14.png (정면 ±52.5°)
이후 build-star-sheet.py로 스프라이트 시트를 만든다.
"""
import math
import sys
from pathlib import Path

import bpy

YAW_STEPS = 48            # 한 바퀴 프레임 수
BLINK_HALF_RANGE = 7      # 정면 기준 좌우 프레임 수
RENDER_SIZE = 512         # 시트에는 절반 크기로 줄여 넣는다
SAMPLES = 128

out_dir = Path(sys.argv[sys.argv.index('--') + 1])
out_dir.mkdir(parents=True, exist_ok=True)

scene = bpy.context.scene
scene.camera = bpy.data.objects['Camera • front']
scene.render.resolution_x = scene.render.resolution_y = RENDER_SIZE
scene.render.resolution_percentage = 100
scene.render.film_transparent = True
scene.render.image_settings.file_format = 'PNG'
scene.render.image_settings.color_mode = 'RGBA'
scene.render.image_settings.color_depth = '8'
scene.cycles.samples = SAMPLES
scene.cycles.use_denoising = True

prefs = bpy.context.preferences.addons['cycles'].preferences
try:
    prefs.compute_device_type = 'METAL'
    prefs.get_devices()
    for device in prefs.devices:
        device.use = True
    scene.cycles.device = 'GPU'
except Exception as error:  # GPU가 없으면 CPU로 렌더한다.
    print('GPU 사용 불가, CPU로 렌더:', error)
    scene.cycles.device = 'CPU'

star = bpy.data.objects['Star']
eyes = [bpy.data.objects['Eye'], bpy.data.objects['Eye.001']]
open_height = [eye.scale.z for eye in eyes]


def render(name: str, step: int, closed: bool) -> None:
    # 양의 각도는 화면 오른쪽을 바라본다.
    star.rotation_euler = (0, 0, math.radians(step * 360 / YAW_STEPS))
    for eye, height in zip(eyes, open_height):
        eye.scale.z = height * (.12 if closed else 1)
    scene.render.filepath = str(out_dir / name)
    bpy.ops.render.render(write_still=True)


only = [arg for arg in sys.argv[sys.argv.index('--') + 2:]]
for step in range(YAW_STEPS):
    name = f'open-{step:02d}.png'
    if not only or name in only:
        render(name, step, closed=False)
for index, step in enumerate(range(-BLINK_HALF_RANGE, BLINK_HALF_RANGE + 1)):
    name = f'blink-{index:02d}.png'
    if not only or name in only:
        render(name, step, closed=True)
print('렌더 완료:', out_dir)
