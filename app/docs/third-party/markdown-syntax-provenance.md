# 코드 블록 추가 문법

2026-11-14. 실행 중 다운로드 없이 `include_str!`로 포함하고, 앱에서 처음 사용할 때 syntect 문법 집합에 추가한다. 기본 문법은 two-face 0.5.2+bat-0.26.1이며 PowerShell과 JSX 문법의 정규식 지원을 위해 syntect의 기본 Oniguruma 엔진을 사용한다. 앱 편집·입력·스타일 처리는 Rust다.

| 파일 | 출처 | 적용 |
|---|---|---|
| Pug.sublime-syntax | [davidrios/pug-tmbundle 고정 소스](https://github.com/davidrios/pug-tmbundle/blob/e165db8b4cd292acb4160875b5622db9298fbc57/Syntaxes/Pug.sublime-syntax) | 변경 없이 포함, MIT |
| Mermaid.sublime-syntax | [SublimeText/Mermaid ST3 고정 소스](https://github.com/SublimeText/Mermaid/blob/378574947ca93ff92c6d8545642a8131ec937259/mermaid.sublime-syntax) | 변경 없이 포함, MIT; 최신 문법 전체 지원을 뜻하지 않음 |
| WebAssembly.sublime-syntax | [bathos/wast-sublime-syntax 고정 소스](https://github.com/bathos/wast-sublime-syntax/blob/6eb7302b23db58052bdc308b046d2ae0bef5e25a/wast.sublime-syntax) | 변경 없이 포함, MIT; 텍스트 형식 WAT/WAST |
| VBScript.sublime-syntax | [TextMate ASP VB.net 고정 소스](https://github.com/textmate/asp.vb.net.tmbundle/blob/0cc4e1dcdb401703e373f32d7f0c8e9220b990dd/Syntaxes/ASP%20VB.net.plist) | TextMate match/begin/end/captures/repository를 syntect 컨텍스트로 변환, 표시 이름·scope·확장자 수정. VB 계열 토큰 강조이며 VBScript 실행·구문 검증기가 아님 |
| MDX.sublime-syntax | Stology 자체 작성 | 기존 Markdown과 TypeScriptReact 문법을 연결하는 어댑터. import/export, JSX 태그, 중괄호 표현식, Markdown 본문 강조. Shiki MDX 문법 전체를 포팅한 것은 아님 |

라이선스 본문은 `app/docs/third-party/`의 `pug-syntax-LICENSE`, `mermaid-syntax-LICENSE`, `wasm-syntax-LICENSE`, `vbscript-textmate-LICENSE.md`에 있으며 앱 빌드에 함께 포함된다. two-face 문법별 원 저작자·라이선스는 같은 폴더의 `two-face-0.5.2+bat-0.26.1-syntax-acknowledgements.md`에 있다.

선택 목록은 Tolaria의 기본·추가 언어 69개를 따른다. 별칭은 저장할 표준 ID로 정규화한다. JSONC/JSONL은 JSON 문법, JSX/TSX는 TypeScriptReact 문법, PostCSS는 CSS 문법, Vue HTML은 Vue 문법, HCL/Terraform은 Terraform 문법을 사용한다. 알 수 없는 외부 언어 ID는 보존하고 단색 코드로 표시한다. 언어 선택과 강조 기능의 동작을 맞춘 것이며 Shiki와 모든 색·토큰 경계가 같다는 뜻은 아니다.

언어를 생략한 코드 블록은 최대 64 KiB 범위에서 JSON·HTML·Python·Bash·TypeScript·JavaScript·SQL·YAML 특징을 로컬에서 추정한다. 명시적으로 선택한 Plain Text와 알 수 없는 언어 ID는 추정으로 덮어쓰지 않는다. 추정과 스타일 계산은 원본 문자열을 변경하지 않는다.
