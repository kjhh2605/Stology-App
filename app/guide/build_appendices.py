#!/usr/bin/env python3
"""Render canonical verification records as offline guide appendices."""
import html
import re
from pathlib import Path

GUIDE = Path(__file__).resolve().parent
SOURCE = GUIDE.parent / 'docs' / 'graph-parity.md'
REPO = GUIDE.parent.parent

def inline(text):
    chunks = []
    last = 0
    for m in re.finditer(r'\[([^\]]+)\]\(([^)]+)\)', text):
        chunks.append(html.escape(text[last:m.start()]))
        label, target = m.groups()
        if target.startswith(('https://', 'http://')):
            chunks.append(f'<a href="{html.escape(target, quote=True)}">{html.escape(label)}</a>')
        else:
            path = (SOURCE.parent / target.split('#')[0]).resolve()
            try: display = str(path.relative_to(REPO))
            except ValueError: display = target
            chunks.append(f'{html.escape(label)} <code>{html.escape(display)}</code>')
        last = m.end()
    chunks.append(html.escape(text[last:]))
    out = ''.join(chunks)
    out = re.sub(r'\*\*(.+?)\*\*', r'<strong>\1</strong>', out)
    return re.sub(r'`([^`]+)`', r'<code>\1</code>', out)

def render(source, output_name, title, return_anchor='map'):
    global SOURCE
    SOURCE = source
    parts = ['<!doctype html><html lang="ko"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>3D 지도 동등성 대응표</title><style>body{font:15px/1.7 -apple-system,sans-serif;color:#25243d;background:#f8f7fc;margin:32px}main{max-width:1500px;margin:auto}h1{font-size:30px}h2{margin-top:40px}a{color:#5146d5}code{font-size:11px;overflow-wrap:anywhere;background:#ece9f7;padding:2px 4px;border-radius:4px}table{border-collapse:collapse;width:100%;background:white;font-size:13px}th,td{border:1px solid #dedbea;text-align:left;padding:12px;vertical-align:top}th{background:#eae6fa}.table{overflow:auto}th:first-child,td:first-child{min-width:250px}th:nth-child(2),td:nth-child(2){min-width:240px}th:nth-child(3),td:nth-child(3){min-width:200px}</style><main><p><a href="../index.html#map">← 가이드로 돌아가기</a></p><p>소스의 <code>local-app/docs/graph-parity.md</code>에서 생성한 오프라인 사본입니다. 코드 참조는 저장소 기준 경로이며 소스 패키지의 원문에서 클릭할 수 있습니다.</p>']
    in_table = False
    for line in SOURCE.read_text().splitlines():
        if line.startswith('|'):
            if re.match(r'^\|[-| :]+\|$', line):
                continue
            cells = [inline(v.strip()) for v in line.strip('|').split('|')]
            tag = 'td' if in_table else 'th'
            if not in_table:
                parts.append('<div class="table"><table>')
            in_table = True
            parts.append('<tr>' + ''.join(f'<{tag}>{v}</{tag}>' for v in cells) + '</tr>')
            continue
        if in_table:
            parts.append('</table></div>')
            in_table = False
        if not line.strip():
            continue
        if line.startswith('#'):
            n = len(line) - len(line.lstrip('#'))
            parts.append(f'<h{n}>{inline(line[n:].strip())}</h{n}>')
        else:
            parts.append('<p>' + inline(line.lstrip('- ')) + '</p>')
    if in_table:
        parts.append('</table></div>')
    parts.append('</main></html>')
    (GUIDE / 'assets' / output_name).write_text('\n'.join(parts).replace('local-app/docs/graph-parity.md', str(source.relative_to(REPO))).replace('<title>3D 지도 동등성 대응표</title>', '<title>' + html.escape(title) + '</title>').replace('../index.html#map', '../index.html#' + return_anchor))
    print('Updated offline appendix:', output_name)

render(GUIDE.parent / 'docs' / 'graph-parity.md', 'graph-parity.html', '3D 지도 동등성 대응표')
render(GUIDE.parent / 'qa' / 'runs' / 'installed-app-2026-11-14.md', 'installed-app-2026-11-14.html', '설치 앱 실제 검증 기록 · 2026-11-14')
render(GUIDE.parent / 'docs' / 'keyboard-shortcuts.md', 'keyboard-shortcuts.html', 'Stology Local 단축키와 Markdown 입력', return_anchor='write')

render(GUIDE.parent / 'qa' / 'pdf-annotation-implementation.md', 'pdf-annotation-implementation.html', 'PDF 필기 구현·검증 기록 · 2026-11-15', return_anchor='verification')
