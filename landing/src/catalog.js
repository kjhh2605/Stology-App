// 기능 목록(언어 중립). 설명 문구는 src/i18n/*.js의 items[id]에, 장면은 src/scenes/*.js에 있다.
// keys는 단축키, alt는 같은 결과를 내는 Markdown 입력. {t}는 각 언어의 '텍스트' 낱말로 바뀐다.
// core가 true인 항목이 기본 탭(핵심)에 보이고, 나머지는 '모두 보기'에서 열린다. 순서가 곧 번호다.
export const noteItems = [
  { id: 'bold', category: 'format', keys: ['⌘', 'B'], alt: [['**{t}**'], ['__{t}__']], core: true },
  { id: 'block-1', category: 'blocks', keys: ['⌘', '⌥', '1'], alt: [['#', 'Space']], core: true },
  { id: 'checklist', category: 'blocks', keys: ['⌘', '⌥', '4'], alt: [['- [ ]', 'Space'], ['- [x]', 'Space']], core: true },
  { id: 'bullet', category: 'blocks', keys: ['⌘', '⌥', '5'], alt: [['-', 'Space'], ['*', 'Space'], ['+', 'Space']], core: true },
  { id: 'selection', category: 'navigate', keys: ['⌘', 'D'], core: true },
  { id: 'markdown-quote', category: 'markdown', keys: ['>', 'Space'], core: true },
  { id: 'italic', category: 'format', keys: ['⌘', 'I'], alt: [['*{t}*'], ['_{t}_']] },
  { id: 'underline', category: 'format', keys: ['⌘', 'U'] },
  { id: 'strikethrough', category: 'format', keys: ['⌘', '⇧', 'S'], alt: [['~~{t}~~']] },
  { id: 'inline-code', category: 'format', keys: ['⌘', 'E'], alt: [['`{t}`']] },
  { id: 'link', category: 'format', keys: ['⌘', 'K'], alt: [['[{t}](https://…)']] },
  { id: 'block-2', category: 'blocks', keys: ['⌘', '⌥', '2'], alt: [['##', 'Space']] },
  { id: 'block-3', category: 'blocks', keys: ['⌘', '⌥', '3'], alt: [['###', 'Space']] },
  { id: 'numbered', category: 'blocks', keys: ['⌘', '⌥', '6'], alt: [['1.', 'Space']] },
  { id: 'block-code', category: 'blocks', keys: ['⌘', '⌥', '8'], alt: [['```', 'python', 'Enter']] },
  { id: 'map-search-focus', category: 'navigate', keys: ['⌘', 'K'] },
];

export const pdfItems = [
  { id: 'highlight', category: 'pdfTools', tool: 'highlight', core: true },
  { id: 'pen', category: 'pdfTools', tool: 'pen', core: true },
  { id: 'text-note', category: 'pdfTools', tool: 'note', core: true },
  { id: 'pdf-area-select', category: 'pdfEdit', tool: 'area-select', core: true },
  { id: 'pdf-analyze', category: 'pdfConvert', chip: 'contextMenu', core: true },
  { id: 'pdf-area-highlight', category: 'pdfTools', tool: 'area' },
  { id: 'pdf-box', category: 'pdfTools', tool: 'box' },
  { id: 'pdf-move', category: 'pdfEdit', tool: 'select' },
  { id: 'pdf-resize', category: 'pdfEdit', tool: 'select' },
  { id: 'pdf-note-edit', category: 'pdfEdit', tool: 'note' },
  { id: 'pdf-delete-many', category: 'pdfEdit', tool: 'area-select', keys: ['⌫'] },
  { id: 'pdf-share', category: 'pdfShare', chip: 'fileMenu' },
  { id: 'pdf-pptx', category: 'pdfConvert', chip: 'contextMenu' },
];

export const groups = { notes: noteItems, pdf: pdfItems };
