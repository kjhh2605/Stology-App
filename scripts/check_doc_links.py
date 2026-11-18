#!/usr/bin/env python3
"""Check relative Markdown link targets; exclude fenced examples, URLs and anchors."""
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
SKIP = {'.git', 'target', 'dist', '.build', 'node_modules', 'private'}
# These are fixture content, DMG-relative instructions, and a verbatim vendor license.
SKIP_FILES = {
    'app/bench/fixture.md',
    'app/docs/PACKAGE-START.md',
    'app/docs/third-party/onig_sys-69.9.3-LICENSE.md',
}
LINK = re.compile(r'\]\((<[^>]+>|[^\s)]+)(?:\s+"[^"]*")?\)')


def main():
    problems = []
    count = 0
    for path in sorted(ROOT.rglob('*.md')):
        if path.relative_to(ROOT).as_posix() in SKIP_FILES or any(
            part in SKIP for part in path.relative_to(ROOT).parts
        ):
            continue
        fence = None
        for number, line in enumerate(path.read_text(encoding='utf-8').splitlines(), 1):
            marker = re.match(r'^\s*(`{3,}|~{3,})', line)
            if marker:
                value = marker.group(1)
                if fence is None:
                    fence = value
                elif value[0] == fence[0] and len(value) >= len(fence):
                    fence = None
                continue
            if fence:
                continue
            for match in LINK.finditer(line):
                target = match.group(1).strip('<>')
                url = urlsplit(target)
                if url.scheme or url.netloc or not url.path:
                    continue
                count += 1
                resolved = (path.parent / unquote(url.path)).resolve()
                if not resolved.is_relative_to(ROOT) or not resolved.exists():
                    problems.append(f'{path.relative_to(ROOT)}:{number}: {target}')
    if problems:
        print('Missing or out-of-repository Markdown targets:\n' + '\n'.join(problems))
        return 1
    print(f'Markdown relative links OK ({count} targets; URLs and anchors not checked)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
