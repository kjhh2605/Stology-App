#!/usr/bin/env python3
"""Record exact registry provenance and include available license texts."""
import json, pathlib, subprocess
root = pathlib.Path(__file__).resolve().parents[1]
rows = {}
for crate in ('core', 'ui'):
    meta = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version=1', '--manifest-path', str(root/crate/'Cargo.toml')]))
    for p in meta['packages']:
        if not p.get('source'): continue
        rows[(p['name'], p['version'])] = p
out = root/'docs'/'third-party'
out.mkdir(parents=True, exist_ok=True)
(out/'markdown-syntax-provenance.md').write_bytes((root/'ui/assets/syntaxes/README.md').read_bytes())
lines = ['# 외부 코드·라이선스 기록', '', 'Cargo.lock의 레지스트리 버전·체크섬으로 고정한다. 레지스트리 배포본에 참조 커밋이 없는 경우 커밋을 추정하지 않는다. 레지스트리 패키지는 수정하지 않았다. Tolaria 및 oh-my-pi 코드를 런타임으로 탑재하지 않는다. 추가 구문 정의의 고정 출처·라이선스와 VB 문법 변환·자체 MDX 어댑터는 [구문 정의 기록](third-party/markdown-syntax-provenance.md)에 기록한다.', '', '## 내장 글꼴', '', 'SUIT v2.0.5 (Regular, Medium, SemiBold, Bold OTF)를 수정 없이 앱에 포함한다. SIL Open Font License 1.1이며 원문은 앱의 `Contents/Resources/Licenses/SUIT-OFL.txt`와 저장소의 [OFL.txt](../assets/fonts/OFL.txt)에 있다. 출처와 고정 커밋은 [폰트 기록](../assets/fonts/README.md)에 명시한다.', '', '## Cargo 의존성', '', '| 패키지 | 버전 | 라이선스 | 참조 커밋 | 출처 | 수정 |', '|---|---|---|---|---|---|']
lines[9:9] = ['## 채팅 차트', '', 'Mermaid 11.12.2 (MIT)를 내장한다. 고정 출처·체크섬은 [런타임 기록](third-party/mermaid-runtime.md), 라이선스 원문은 [LICENSE](third-party/mermaid-runtime-LICENSE)에 있다.', '']
for (name, version), p in sorted(rows.items()):
    base = pathlib.Path(p['manifest_path']).parent
    vcs = base/'.cargo_vcs_info.json'
    commit = json.loads(vcs.read_text()).get('git',{}).get('sha1','미기재') if vcs.exists() else '레지스트리 배포본 · 커밋 미기재'
    lines.append(f'| {name} | {version} | {p.get("license") or "확인 필요"} | {commit} | {p.get("repository") or p["source"]} | 없음 |')
    for f in sorted(base.iterdir()):
        if f.is_file() and f.name.upper().startswith(('LICENSE','COPYING','NOTICE')):
            (out/f'{name}-{version}-{f.name}').write_bytes(f.read_bytes())
(root/'docs'/'THIRD-PARTY.md').write_text('\n'.join(lines)+'\n')
print(f'{len(rows)} dependency versions recorded')
