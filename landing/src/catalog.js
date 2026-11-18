// Selected landing features; the complete app shortcut reference remains in app/docs/keyboard-shortcuts.md.
export const shortcuts = [
  {
    "id": "bold",
    "label": "굵게",
    "title": "굵게",
    "body": "한 문단 안의 내용을 선택해 굵게 표시합니다. 서식이 적용된 텍스트 전체를 선택하고 다시 누르면 해제돼요.",
    "category": "텍스트 서식",
    "media": "bold",
    "icon": "note",
    "context": "텍스트 서식 · 굵게",
    "keys": [
      "⌘",
      "B"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 47
    },
    "scene": {
      "kind": "format",
      "action": "bold",
      "before": "기억은 경험을 연결하는 과정입니다.",
      "after": "**기억은 경험을 연결하는 과정입니다.**",
      "trigger": "⌘ + B",
      "steps": [
        "한 문단 안의 내용을 선택해 굵게 표시합니다. 서식이 적용된 텍스트 전체를 선택하고 다시 누르면 해제돼요."
      ]
    },
    "keyAlternatives": [
      [
        "**텍스트**"
      ],
      [
        "__텍스트__"
      ]
    ]
  },
  {
    "id": "italic",
    "label": "기울임",
    "title": "기울임",
    "body": "선택한 내용을 기울여 표시합니다. 같은 단축키로 서식을 해제할 수 있어요.",
    "category": "텍스트 서식",
    "media": "inline-format",
    "icon": "note",
    "context": "텍스트 서식 · 기울임",
    "keys": [
      "⌘",
      "I"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 48
    },
    "scene": {
      "kind": "format",
      "action": "italic",
      "before": "기억은 경험을 연결하는 과정입니다.",
      "after": "*기억은 경험을 연결하는 과정입니다.*",
      "trigger": "⌘ + I",
      "steps": [
        "선택한 내용을 기울여 표시합니다. 같은 단축키로 서식을 해제할 수 있어요."
      ]
    },
    "keyAlternatives": [
      [
        "*텍스트*"
      ],
      [
        "_텍스트_"
      ]
    ]
  },
  {
    "id": "underline",
    "label": "밑줄",
    "title": "밑줄",
    "body": "선택한 내용에 밑줄을 긋습니다. 같은 단축키를 다시 누르면 밑줄이 지워져요.",
    "category": "텍스트 서식",
    "media": "inline-format",
    "icon": "note",
    "context": "텍스트 서식 · 밑줄",
    "keys": [
      "⌘",
      "U"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 49
    },
    "scene": {
      "kind": "format",
      "action": "underline",
      "before": "기억은 경험을 연결하는 과정입니다.",
      "after": "<u>기억은 경험을 연결하는 과정입니다.</u>",
      "trigger": "⌘ + U",
      "steps": [
        "선택한 내용에 밑줄을 긋습니다. 같은 단축키를 다시 누르면 밑줄이 지워져요."
      ]
    },
    "keyAlternatives": []
  },
  {
    "id": "strikethrough",
    "label": "취소선",
    "title": "취소선",
    "body": "선택한 내용에 취소선을 긋습니다. 수정한 내용이나 제외할 항목을 남겨 둘 때 사용해 보세요.",
    "category": "텍스트 서식",
    "media": "inline-format",
    "icon": "note",
    "context": "텍스트 서식 · 취소선",
    "keys": [
      "⌘",
      "⇧",
      "S"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 50
    },
    "scene": {
      "kind": "format",
      "action": "strikethrough",
      "before": "기억은 경험을 연결하는 과정입니다.",
      "after": "~~기억은 경험을 연결하는 과정입니다.~~",
      "trigger": "⌘ + ⇧ + S",
      "steps": [
        "선택한 내용에 취소선을 긋습니다. 수정한 내용이나 제외할 항목을 남겨 둘 때 사용해 보세요."
      ]
    },
    "keyAlternatives": [
      [
        "~~텍스트~~"
      ]
    ]
  },
  {
    "id": "inline-code",
    "label": "인라인 코드",
    "title": "인라인 코드",
    "body": "선택한 내용을 인라인 코드로 감쌉니다. 코드 전체를 선택하고 다시 누르면 원래 글로 돌아와요.",
    "category": "텍스트 서식",
    "media": "inline-format",
    "icon": "note",
    "context": "텍스트 서식 · 인라인 코드",
    "keys": [
      "⌘",
      "E"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 51
    },
    "scene": {
      "kind": "format",
      "action": "inline-code",
      "before": "기억은 경험을 연결하는 과정입니다.",
      "after": "`기억은 경험을 연결하는 과정입니다.`",
      "trigger": "⌘ + E",
      "steps": [
        "선택한 내용을 인라인 코드로 감쌉니다. 코드 전체를 선택하고 다시 누르면 원래 글로 돌아와요."
      ]
    },
    "keyAlternatives": [
      [
        "`텍스트`"
      ]
    ]
  },
  {
    "id": "link",
    "label": "선택한 글에 링크",
    "title": "선택한 글에 링크",
    "body": "한 문단 안의 글을 선택한 뒤 주소를 입력합니다. https://, http://, mailto:로 시작하는 주소를 사용할 수 있어요.",
    "category": "텍스트 서식",
    "media": "link",
    "icon": "note",
    "context": "텍스트 서식 · 선택한 글에 링크",
    "keys": [
      "⌘",
      "K"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 52
    },
    "scene": {
      "kind": "link",
      "action": "link",
      "before": "수업 참고 자료",
      "after": "[수업 참고 자료](https://example.com)",
      "trigger": "⌘ + K",
      "steps": [
        "한 문단 안의 글을 선택한 뒤 주소를 입력합니다. https://, http://, mailto:로 시작하는 주소를 사용할 수 있어요."
      ]
    },
    "keyAlternatives": [
      [
        "[이름](https://example.com)"
      ]
    ]
  },
  {
    "id": "block-1",
    "label": "제목 1",
    "title": "제목 1",
    "body": "현재 줄이나 선택한 줄을 큰 제목으로 바꿉니다. 줄 시작에 #을 쓰고 Space를 눌러도 제목 1을 만들 수 있어요.",
    "category": "문단과 제목",
    "media": "block-headings",
    "icon": "note",
    "context": "문단과 제목 · 제목 1",
    "keys": [
      "⌘",
      "⌥",
      "1"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 67
    },
    "scene": {
      "kind": "block",
      "action": "block-1",
      "before": "기억과 학습",
      "after": "# 기억과 학습",
      "trigger": "⌘ + ⌥ + 1",
      "steps": [
        "현재 줄이나 선택한 줄을 큰 제목으로 바꿉니다. 줄 시작에 #을 쓰고 Space를 눌러도 제목 1을 만들 수 있어요."
      ]
    },
    "keyAlternatives": [
      [
        "#",
        "Space"
      ]
    ]
  },
  {
    "id": "block-2",
    "label": "제목 2",
    "title": "제목 2",
    "body": "현재 줄이나 선택한 줄을 중간 제목으로 바꿉니다. 줄 시작에 ##을 쓰고 Space를 눌러도 제목 2를 만들 수 있어요.",
    "category": "문단과 제목",
    "media": "block-headings",
    "icon": "note",
    "context": "문단과 제목 · 제목 2",
    "keys": [
      "⌘",
      "⌥",
      "2"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 68
    },
    "scene": {
      "kind": "block",
      "action": "block-2",
      "before": "기억과 학습",
      "after": "## 기억과 학습",
      "trigger": "⌘ + ⌥ + 2",
      "steps": [
        "현재 줄이나 선택한 줄을 중간 제목으로 바꿉니다. 줄 시작에 ##을 쓰고 Space를 눌러도 제목 2를 만들 수 있어요."
      ]
    },
    "keyAlternatives": [
      [
        "##",
        "Space"
      ]
    ]
  },
  {
    "id": "block-3",
    "label": "제목 3",
    "title": "제목 3",
    "body": "현재 줄이나 선택한 줄을 작은 제목으로 바꿉니다. 줄 시작에 ###을 쓰고 Space를 눌러도 제목 3을 만들 수 있어요.",
    "category": "문단과 제목",
    "media": "block-headings",
    "icon": "note",
    "context": "문단과 제목 · 제목 3",
    "keys": [
      "⌘",
      "⌥",
      "3"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 69
    },
    "scene": {
      "kind": "block",
      "action": "block-3",
      "before": "기억과 학습",
      "after": "### 기억과 학습",
      "trigger": "⌘ + ⌥ + 3",
      "steps": [
        "현재 줄이나 선택한 줄을 작은 제목으로 바꿉니다. 줄 시작에 ###을 쓰고 Space를 눌러도 제목 3을 만들 수 있어요."
      ]
    },
    "keyAlternatives": [
      [
        "###",
        "Space"
      ]
    ]
  },
  {
    "id": "checklist",
    "label": "체크리스트",
    "title": "체크리스트",
    "body": "현재 줄이나 선택한 줄을 체크리스트로 바꿉니다. 줄 시작에 - [ ]와 Space를 입력하고 내용을 써도 만들 수 있어요.",
    "category": "문단과 제목",
    "media": "checklist",
    "icon": "note",
    "context": "문단과 제목 · 체크리스트",
    "keys": [
      "⌘",
      "⌥",
      "4"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 70
    },
    "scene": {
      "kind": "block",
      "action": "checklist",
      "before": "오늘 배운 개념 설명하기",
      "after": "- [ ] 오늘 배운 개념 설명하기",
      "trigger": "⌘ + ⌥ + 4",
      "steps": [
        "현재 줄이나 선택한 줄을 체크리스트로 바꿉니다. 줄 시작에 - [ ]와 Space를 입력하고 내용을 써도 만들 수 있어요."
      ]
    },
    "keyAlternatives": [
      [
        "- [ ]",
        "Space"
      ],
      [
        "- [x]",
        "Space"
      ]
    ]
  },
  {
    "id": "bullet",
    "label": "불릿 목록",
    "title": "불릿 목록",
    "body": "현재 줄이나 선택한 줄을 불릿 목록으로 바꿉니다. 줄 시작에 -, *, + 중 하나를 쓰고 Space를 눌러도 돼요.",
    "category": "문단과 제목",
    "media": "block-lists",
    "icon": "note",
    "context": "문단과 제목 · 불릿 목록",
    "keys": [
      "⌘",
      "⌥",
      "5"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 71
    },
    "scene": {
      "kind": "block",
      "action": "bullet",
      "before": "오늘 배운 개념 설명하기",
      "after": "- 오늘 배운 개념 설명하기",
      "trigger": "⌘ + ⌥ + 5",
      "steps": [
        "현재 줄이나 선택한 줄을 불릿 목록으로 바꿉니다. 줄 시작에 -, *, + 중 하나를 쓰고 Space를 눌러도 돼요."
      ]
    },
    "keyAlternatives": [
      [
        "-",
        "Space"
      ],
      [
        "*",
        "Space"
      ],
      [
        "+",
        "Space"
      ]
    ]
  },
  {
    "id": "numbered",
    "label": "번호 목록",
    "title": "번호 목록",
    "body": "현재 줄이나 선택한 줄을 번호 목록으로 바꿉니다. 줄 시작에 1.을 쓰고 Space를 눌러도 목록을 만들 수 있어요.",
    "category": "문단과 제목",
    "media": "block-lists",
    "icon": "note",
    "context": "문단과 제목 · 번호 목록",
    "keys": [
      "⌘",
      "⌥",
      "6"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 72
    },
    "scene": {
      "kind": "block",
      "action": "numbered",
      "before": "오늘 배운 개념 설명하기",
      "after": "1. 오늘 배운 개념 설명하기",
      "trigger": "⌘ + ⌥ + 6",
      "steps": [
        "현재 줄이나 선택한 줄을 번호 목록으로 바꿉니다. 줄 시작에 1.을 쓰고 Space를 눌러도 목록을 만들 수 있어요."
      ]
    },
    "keyAlternatives": [
      [
        "1.",
        "Space"
      ]
    ]
  },
  {
    "id": "block-code",
    "label": "코드 블록",
    "title": "코드 블록",
    "body": "현재 줄이나 선택한 줄을 코드 블록으로 바꿉니다. 직접 만들 때는 줄 시작에 백틱 세 개와 언어 이름(예: python)을 쓰고 Enter를 누르세요.",
    "category": "문단과 제목",
    "media": "block-code",
    "icon": "note",
    "context": "문단과 제목 · 코드 블록",
    "keys": [
      "⌘",
      "⌥",
      "8"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 73
    },
    "scene": {
      "kind": "block",
      "action": "block-code",
      "before": "오늘 배운 개념 설명하기",
      "after": "```text\n오늘 배운 개념 설명하기\n```",
      "trigger": "⌘ + ⌥ + 8",
      "steps": [
        "현재 줄이나 선택한 줄을 코드 블록으로 바꿉니다. 직접 만들 때는 줄 시작에 백틱 세 개와 언어 이름(예: python)을 쓰고 Enter를 누르세요."
      ]
    },
    "keyAlternatives": [
      [
        "```",
        "python",
        "Enter"
      ],
      [
        "```\n코드\n```"
      ]
    ],
    "note": "Markdown 원문에서는 코드 앞뒤에 백틱 세 개를 각각 한 줄씩 놓습니다."
  },
  {
    "id": "markdown-quote",
    "label": "인용 직접 입력",
    "title": "인용 직접 입력",
    "body": "줄 시작에 >를 쓰고 Space를 누르면 인용이 됩니다. 수업에서 기억하고 싶은 문장을 따로 담아 보세요.",
    "category": "Markdown 입력",
    "media": "markdown-input",
    "icon": "note",
    "context": "Markdown 입력 · 인용 직접 입력",
    "tag": "> + Space",
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 134
    },
    "scene": {
      "kind": "markdown",
      "action": "markdown-quote",
      "before": "> ",
      "after": "> 기억은 경험을 연결합니다.",
      "trigger": "> + Space",
      "steps": [
        "줄 시작에 >를 쓰고 Space를 누르면 인용이 됩니다. 수업에서 기억하고 싶은 문장을 따로 담아 보세요."
      ]
    },
    "keyAlternatives": [],
    "keys": [
      ">",
      "Space"
    ]
  },
  {
    "id": "selection",
    "label": "선택 범위 확장",
    "title": "선택 범위 확장",
    "body": "누를 때마다 단어 → 괄호·인용·서식 → 문장·줄 → 문단·블록 → 문서로 넓힙니다. 현재 글에 없는 단계는 건너뛰어요.",
    "category": "선택과 탐색",
    "media": "selection-extended",
    "icon": "selection",
    "context": "선택과 탐색 · 선택 범위 확장",
    "keys": [
      "⌘",
      "D"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 19
    },
    "scene": {
      "kind": "selection",
      "action": "selection",
      "before": "오늘 배운 개념(단기 기억)을 내 경험과 연결해 봅니다.\n친구 이름을 여러 번 불러 봅니다.",
      "after": "오늘 배운 개념(단기 기억)을 내 경험과 연결해 봅니다.\n친구 이름을 여러 번 불러 봅니다.",
      "trigger": "⌘ + D",
      "steps": [
        "단어를 선택합니다.",
        "괄호 안의 의미 단위로 넓힙니다.",
        "문장과 문단, 문서로 넓힙니다."
      ]
    },
    "keyAlternatives": []
  },
  {
    "id": "map-search-focus",
    "label": "지도 검색창으로 이동",
    "title": "지도 검색창으로 이동",
    "body": "본문에 선택한 글이 없고 지도 검색창이 열려 있다면 검색창으로 이동합니다. 노트에서 글을 선택했을 때는 링크 추가로 동작해요.",
    "category": "선택과 탐색",
    "media": "title-search",
    "icon": "selection",
    "context": "선택과 탐색 · 지도 검색창으로 이동",
    "keys": [
      "⌘",
      "K"
    ],
    "source": {
      "path": "app/docs/keyboard-shortcuts.md",
      "line": 56
    },
    "scene": {
      "kind": "search",
      "action": "map-search-focus",
      "before": "지식지도 · 기억",
      "after": "검색창에 커서 · 기억",
      "trigger": "⌘ + K",
      "steps": [
        "본문에 선택한 글이 없고 지도 검색창이 열려 있다면, 검색창으로 이동합니다. 노트에서 글을 선택했을 때는 링크 추가로 동작해요."
      ]
    },
    "keyAlternatives": []
  }
];

export const pdfTools = [
  {
    "id": "highlight",
    "label": "본문 텍스트 강조",
    "title": "본문 텍스트 강조",
    "body": "읽기 도구에서 먼저 본문을 드래그해 선택한 뒤 강조 버튼을 누릅니다. 선택한 구절이 노란색으로 표시돼요.",
    "category": "필기 도구",
    "tag": "텍스트 강조",
    "media": "highlight",
    "icon": "pen",
    "context": "PDF · 본문 텍스트 강조",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 1094
    },
    "scene": {
      "kind": "pdf",
      "action": "highlight",
      "before": "반복과 연결은 기억에 도움을 줍니다.",
      "after": "선택한 문장에 노란 형광펜",
      "trigger": "텍스트 강조",
      "steps": [
        "읽기 도구에서 문장을 드래그합니다.",
        "강조 버튼을 누릅니다.",
        "텍스트 줄을 따라 노란 강조가 남습니다."
      ]
    }
  },
  {
    "id": "pdf-area-highlight",
    "label": "영역 강조",
    "title": "영역 강조",
    "body": "강조할 곳을 사각형으로 드래그합니다. 텍스트 선택 없이도 그림이나 페이지의 일부를 노란 면으로 표시할 수 있어요.",
    "category": "필기 도구",
    "tag": "영역 강조",
    "media": "pdf-area-highlight",
    "icon": "pen",
    "context": "PDF · 영역 강조",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 915
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-area-highlight",
      "before": "오늘의 핵심 문단",
      "after": "문단 위에 반투명 노란 사각형",
      "trigger": "영역 강조",
      "steps": [
        "영역 강조 도구를 고릅니다.",
        "오늘의 핵심 문단 위를 사각형으로 드래그합니다.",
        "드래그한 영역에 노란 면이 남습니다."
      ]
    }
  },
  {
    "id": "pdf-box",
    "label": "박스 그리기",
    "title": "박스 그리기",
    "body": "박스 도구로 사각형을 드래그해 중요한 내용을 둘러쌉니다. 클릭만 하면 박스를 만들지 않아요.",
    "category": "필기 도구",
    "tag": "박스",
    "media": "pdf-box",
    "icon": "pen",
    "context": "PDF · 박스 그리기",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 916
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-box",
      "before": "오늘의 핵심 문단",
      "after": "문단을 둘러싼 테두리 박스",
      "trigger": "박스",
      "steps": [
        "박스 도구를 고릅니다.",
        "문단을 둘러싸도록 드래그합니다."
      ]
    }
  },
  {
    "id": "text-note",
    "label": "메모 남기기",
    "title": "메모 남기기",
    "body": "메모 도구로 페이지를 누르고 글을 적습니다. 수업에서 들은 예시나 내 생각을 PDF 옆에 남겨보세요.",
    "category": "필기 도구",
    "tag": "메모",
    "media": "text-note",
    "icon": "pen",
    "context": "PDF · 메모 남기기",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 917
    },
    "scene": {
      "kind": "pdf",
      "action": "text-note",
      "before": "비어 있는 페이지 여백",
      "after": "내 경험과 연결하면 기억하기 쉬워요.",
      "trigger": "메모",
      "steps": [
        "메모 도구로 여백을 클릭합니다.",
        "노란 편집 상자에 글을 입력합니다.",
        "편집을 마치면 페이지에 파란 글씨로 남습니다."
      ]
    }
  },
  {
    "id": "pen",
    "label": "펜으로 자유 필기",
    "title": "펜으로 자유 필기",
    "body": "펜 도구로 밑줄이나 화살표를 그립니다. 포인터를 움직인 경로가 파란 필기로 남아요.",
    "category": "필기 도구",
    "tag": "펜",
    "media": "pen",
    "icon": "pen",
    "context": "PDF · 펜으로 자유 필기",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 918
    },
    "scene": {
      "kind": "pdf",
      "action": "pen",
      "before": "친구의 이름을 여러 번 불러 보기",
      "after": "문장 아래 파란 손그림 밑줄",
      "trigger": "펜",
      "steps": [
        "펜 도구를 고릅니다.",
        "문장 아래를 자유롭게 드래그합니다."
      ]
    }
  },
  {
    "id": "pdf-move",
    "label": "필기 옮기기",
    "title": "필기 옮기기",
    "body": "선택·이동 도구에서 필기를 드래그해 위치를 바꿉니다. PDF 원래 본문은 이동하지 않아요.",
    "category": "선택과 편집",
    "tag": "드래그",
    "media": "pdf-move",
    "icon": "selection",
    "context": "PDF · 필기 옮기기",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 1002
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-move",
      "before": "여백 위쪽의 메모",
      "after": "여백 아래쪽으로 이동한 메모",
      "trigger": "드래그",
      "steps": [
        "필기를 선택합니다.",
        "필기 안쪽을 잡고 새 위치로 옮깁니다."
      ]
    }
  },
  {
    "id": "pdf-resize",
    "label": "필기 크기 조절",
    "title": "필기 크기 조절",
    "body": "선택한 필기의 오른쪽 아래 핸들을 드래그해 크기를 바꿉니다.",
    "category": "선택과 편집",
    "tag": "우하단 핸들",
    "media": "pdf-resize",
    "icon": "selection",
    "context": "PDF · 필기 크기 조절",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 904
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-resize",
      "before": "작은 박스 선택",
      "after": "오른쪽 아래로 커진 박스",
      "trigger": "우하단 핸들",
      "steps": [
        "박스를 선택합니다.",
        "오른쪽 아래 핸들을 드래그합니다."
      ]
    }
  },
  {
    "id": "pdf-area-select",
    "label": "여러 필기 함께 선택",
    "title": "여러 필기 함께 선택",
    "body": "점선 사각형 도구로 범위를 그립니다. 사각형에 완전히 들어온 앱 필기를 여러 페이지에 걸쳐 선택해요.",
    "category": "선택과 편집",
    "tag": "영역 선택",
    "media": "pdf-area-select",
    "icon": "selection",
    "context": "PDF · 여러 필기 함께 선택",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 686
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-area-select",
      "before": "본문 위의 노란 강조와 파란 밑줄",
      "after": "점선 범위 안의 노란 강조·파란 밑줄 선택",
      "trigger": "영역 선택",
      "steps": [
        "영역 선택 도구를 고릅니다.",
        "노란 강조와 파란 밑줄 전체가 들어오도록 사각형을 드래그합니다.",
        "완전히 포함된 두 필기에 선택 테두리가 생깁니다."
      ]
    }
  },
  {
    "id": "pdf-note-edit",
    "label": "메모 다시 고치기",
    "title": "메모 다시 고치기",
    "body": "선택·이동 도구에서 메모를 두 번 누르면 글을 다시 편집합니다.",
    "category": "선택과 편집",
    "tag": "더블클릭",
    "media": "pdf-note-edit",
    "icon": "selection",
    "context": "PDF · 메모 다시 고치기",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 899
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-note-edit",
      "before": "기억은 반복으로 남아요.",
      "after": "기억은 반복과 연결로 남아요.",
      "trigger": "더블클릭",
      "steps": [
        "선택·이동 도구로 메모를 더블클릭합니다.",
        "내용을 고칩니다.",
        "편집을 마칩니다."
      ]
    }
  },
  {
    "id": "pdf-delete-many",
    "label": "여러 필기 한 번에 삭제",
    "title": "여러 필기 한 번에 삭제",
    "body": "영역 선택한 필기를 한꺼번에 지웁니다. 한 번의 실행 취소로 삭제 묶음을 되돌릴 수 있어요.",
    "category": "선택과 편집",
    "tag": "영역 선택 → 삭제",
    "media": "pdf-delete-many",
    "icon": "selection",
    "context": "PDF · 여러 필기 한 번에 삭제",
    "source": {
      "path": "app/ui/src/pdf_annotations.rs",
      "line": 1055
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-delete-many",
      "before": "노란 강조와 파란 밑줄이 함께 선택됨",
      "after": "두 필기를 함께 삭제 후 실행 취소로 복원",
      "trigger": "영역 선택 → 삭제",
      "steps": [
        "점선 사각형으로 노란 강조와 파란 밑줄을 선택합니다.",
        "두 필기를 한꺼번에 삭제합니다.",
        "실행 취소 한 번으로 두 필기를 모두 복원합니다."
      ]
    }
  },
  {
    "id": "pdf-share",
    "label": "최신 PDF 공유",
    "title": "최신 PDF 공유",
    "body": "파일 메뉴의 PDF 공유를 선택합니다. 최신 필기가 저장된 다음 macOS 공유 선택기를 열어요.",
    "category": "저장과 공유",
    "tag": "PDF 공유…",
    "media": "pdf-share",
    "icon": "note",
    "context": "PDF · 최신 PDF 공유",
    "source": {
      "path": "app/ui/src/main.rs",
      "line": 1840
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-share",
      "before": "필기를 추가한 작업 PDF",
      "after": "저장 완료 후 macOS 공유 선택기",
      "trigger": "PDF 공유…",
      "steps": [
        "파일 메뉴에서 PDF 공유를 고릅니다.",
        "최신 필기 저장을 기다립니다.",
        "공유할 앱이나 방법을 고릅니다."
      ]
    }
  },
  {
    "id": "pdf-pptx",
    "label": "PPTX를 PDF로 필기",
    "title": "PPTX를 PDF로 필기",
    "body": "PPTX 자료 메뉴에서 PDF로 필기를 선택합니다. 연결된 필기 PDF가 있으면 이어서 열고 없으면 같은 폴더에 새 PDF를 만듭니다. 설치된 LibreOffice가 필요해요.",
    "category": "변환과 분석",
    "tag": "PDF로 필기",
    "media": "pdf-pptx",
    "icon": "note",
    "context": "PDF · PPTX를 PDF로 필기",
    "source": {
      "path": "app/ui/src/document_tree.rs",
      "line": 586
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-pptx",
      "before": "강의자료.pptx",
      "after": "같은 폴더의 필기용 PDF",
      "trigger": "PDF로 필기",
      "steps": [
        "PPTX 자료 메뉴를 엽니다.",
        "PDF로 필기를 선택합니다.",
        "만들어진 PDF에서 필기를 시작합니다."
      ]
    }
  },
  {
    "id": "pdf-analyze",
    "label": "필기한 PDF 분석",
    "title": "필기한 PDF 분석",
    "body": "자료 분석을 직접 요청하면 최신 필기를 저장한 뒤 현재 PDF를 연결한 AI 공급자에게 보냅니다. 파일을 읽거나 필기하는 것만으로 분석하지 않아요.",
    "category": "변환과 분석",
    "tag": "분석 요청",
    "media": "pdf-analyze",
    "icon": "note",
    "context": "PDF · 필기한 PDF 분석",
    "source": {
      "path": "app/ui/src/main.rs",
      "line": 2268
    },
    "scene": {
      "kind": "pdf",
      "action": "pdf-analyze",
      "before": "필기한 PDF · 분석 미완료",
      "after": "저장 완료 후 선택한 AI로 분석 요청",
      "trigger": "분석 요청",
      "steps": [
        "분석할 PDF를 선택합니다.",
        "분석을 직접 요청합니다.",
        "최신 필기 저장 후 현재 PDF를 전달합니다."
      ]
    }
  }
];

export const shortcutNotes = [];

export const shortcutLimits = [];

export const pdfNotes = [
  {
    "category": "저장과 공유",
    "title": "작업 PDF와 외부 원본",
    "body": "필기는 앱의 작업 PDF에 저장하며 처음 가져온 외부 원본을 자동으로 덮어쓰지 않습니다. 저장 중과 PDF에 저장됨을 구분해 확인하세요.",
    "source": {
      "path": "app/docs/product-policy.md",
      "line": 103
    }
  }
];

export const pdfLimits = [];
