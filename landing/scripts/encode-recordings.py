#!/usr/bin/env python3
"""Encode real Stology window recordings as silent, caption-free landing loops.

New captures are read from /tmp/stology-landing-capture/passive/<id>.mov only
after <id>.ready is present. --legacy also imports six previously reviewed
native captures. Source files remain untouched. Unknown framing fails closed.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

LANDING = Path(__file__).resolve().parents[1]
CAPTURES = Path('/tmp/stology-landing-capture')
LEGACY = {
    'bold': 'bold.mov',
    'checklist': 'checklist3.mov',
    'selection': 'selection2.mov',
    'highlight': 'highlight.mov',
    'text-note': 'text-note2.mov',
    'pen': 'pen.mov',
}
FRAMING = {
    (2784, 1824): 'crop=2560:1600:112:76,scale=1280:800',
    (1392, 912): 'crop=1280:800:56:38',
    (2560, 1600): 'scale=1280:800',
    (1280, 800): 'null',
}
NOTE_IDS = {'bold', 'italic', 'underline', 'strikethrough', 'inline-code', 'link',
            'block-1', 'block-2', 'block-3', 'checklist', 'bullet', 'numbered',
            'block-code', 'markdown-quote', 'selection'}
PDF_ANNOTATION_IDS = {'highlight', 'pdf-area-highlight', 'pdf-box', 'text-note', 'pen',
    'pdf-move', 'pdf-resize', 'pdf-area-select', 'pdf-note-edit', 'pdf-delete-many'}
CATALOG_ORDER = ['bold', 'italic', 'underline', 'strikethrough', 'inline-code', 'link',
    'block-1', 'block-2', 'block-3', 'checklist', 'bullet', 'numbered', 'block-code',
    'markdown-quote', 'selection', 'map-search-focus', 'highlight', 'pdf-area-highlight',
    'pdf-box', 'text-note', 'pen', 'pdf-move', 'pdf-resize', 'pdf-area-select',
    'pdf-note-edit', 'pdf-delete-many', 'pdf-share', 'pdf-pptx', 'pdf-analyze']


def framing_for(item_id: str, source: Path, dimensions: tuple[int, int]) -> str:
    if dimensions not in FRAMING:
        raise RuntimeError(f'{item_id}: inspect unknown framing {dimensions} before encoding')
    if dimensions == (2784, 1824) and item_id in NOTE_IDS:
        # New native recordings put the example near the top of the editor.
        # The three reused note captures place the action farther down the page.
        top = 406 if source.parent == CAPTURES else 166
        return f'crop=1384:864:556:{top},scale=1280:800'
    if dimensions == (2784, 1824) and item_id in PDF_ANNOTATION_IDS:
        # Include the native toolbar, annotated page, and the document list.
        return 'crop=2000:1250:556:336,scale=1280:800'
    return FRAMING[dimensions]


def run(command: list[str]) -> str:
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(result.stderr[-4000:])
    return result.stdout


def probe(path: Path) -> dict:
    return json.loads(run(['ffprobe', '-v', 'error', '-show_entries',
        'format=duration:stream=codec_type,codec_name,width,height,avg_frame_rate',
        '-of', 'json', str(path)]))


def signature(path: Path) -> dict:
    stat = path.stat()
    return {'path': str(path), 'bytes': stat.st_size, 'mtime_ns': stat.st_mtime_ns}


def encode(item_id: str, source: Path, output: Path, qa: Path, force: bool) -> dict:
    current = signature(source)
    report_path = qa / f'{item_id}.json'
    video_path, poster_path = output / f'{item_id}.mp4', output / f'{item_id}.jpg'
    source_info = probe(source)
    video = next((stream for stream in source_info['streams']
                  if stream['codec_type'] == 'video'), None)
    if video is None:
        raise RuntimeError(f'{item_id}: no video stream')
    dimensions = video['width'], video['height']
    framing = framing_for(item_id, source, dimensions)
    duration = float(source_info['format']['duration'])
    edit = None
    result_source = source.parent / 'pdf-analyze-result.mov'
    if item_id == 'pdf-share':
        edit = {'segments': [{'input': 0, 'start': 5.0, 'end': duration}],
                'reason': 'Remove idle time before the native sharing picker opens.'}
    elif item_id == 'pdf-analyze' and result_source.is_file() and result_source.with_suffix('.ready').is_file():
        result_info = probe(result_source)
        result_video = next(stream for stream in result_info['streams'] if stream['codec_type'] == 'video')
        result_framing = framing_for(item_id, result_source, (result_video['width'], result_video['height']))
        result_duration = float(result_info['format']['duration'])
        edit = {'segments': [
                    {'input': 0, 'start': 0, 'end': 3},
                    {'input': 0, 'start': 10.5, 'end': 13.5},
                    {'input': 0, 'start': 21.5, 'end': duration},
                    {'input': 1, 'start': max(0, result_duration - 3), 'end': result_duration}],
                'additionalSource': signature(result_source), 'additionalFilter': result_framing,
                'reason': 'Remove idle waits; append the actual completion of the same native analysis job.'}
    if not force and report_path.exists() and video_path.exists() and poster_path.exists():
        previous = json.loads(report_path.read_text())
        if previous.get('source') == current and previous.get('filter') == framing and previous.get('edit') == edit:
            return {'id': item_id, 'status': 'unchanged', 'duration': previous['duration']}
    if duration < 1:
        raise RuntimeError(f'{item_id}: capture too short ({duration}s)')
    with tempfile.TemporaryDirectory(prefix=f'{item_id}-', dir=output) as temporary:
        temp = Path(temporary)
        mp4, jpg = temp / 'video.mp4', temp / 'poster.jpg'
        command = ['ffmpeg', '-y', '-i', str(source)]
        if edit:
            if 'additionalSource' in edit:
                command += ['-i', str(result_source)]
            chains = []
            for index, segment in enumerate(edit['segments']):
                crop = framing if segment['input'] == 0 else edit['additionalFilter']
                chains.append(f"[{segment['input']}:v]trim=start={segment['start']}:end={segment['end']},"
                    f"setpts=PTS-STARTPTS,{crop},fps=30,setsar=1[s{index}]")
            labels = ''.join(f'[s{index}]' for index in range(len(chains)))
            chains.append(f'{labels}concat=n={len(chains)}:v=1:a=0[outv]')
            command += ['-filter_complex', ';'.join(chains), '-map', '[outv]']
        else:
            command += ['-map', '0:v:0', '-vf', framing]
        run(command + ['-an', '-sn', '-dn', '-c:v', 'libx264', '-crf', '21', '-preset', 'fast',
             '-threads', '2', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', str(mp4)])
        encoded = probe(mp4)
        streams = encoded['streams']
        if len(streams) != 1 or streams[0]['codec_type'] != 'video':
            raise RuntimeError(f'{item_id}: output must contain only a video stream')
        if (streams[0]['width'], streams[0]['height'], streams[0]['codec_name']) != (1280, 800, 'h264'):
            raise RuntimeError(f'{item_id}: unexpected output format')
        actual_duration = float(encoded['format']['duration'])
        run(['ffmpeg', '-y', '-ss', str(max(0, actual_duration - 1)), '-i', str(mp4),
             '-frames:v', '1', '-q:v', '2', str(jpg)])
        run(['ffmpeg', '-y', '-i', str(mp4), '-vf',
             f'fps=6/{actual_duration},scale=480:300,tile=3x2', '-frames:v', '1',
             '-q:v', '2', str(temp / 'contact.jpg')])
        if signature(source) != current:
            raise RuntimeError(f'{item_id}: source changed during encoding')
        if edit and 'additionalSource' in edit and signature(result_source) != edit['additionalSource']:
            raise RuntimeError(f'{item_id}: completion source changed during encoding')
        digest = hashlib.sha256(mp4.read_bytes()).hexdigest()
        os.replace(mp4, video_path)
        os.replace(jpg, poster_path)
        os.replace(temp / 'contact.jpg', qa / f'{item_id}-contact.jpg')
    report = {
        'id': item_id, 'status': 'encoded', 'source': current,
        'sourceDimensions': list(dimensions), 'filter': framing,
        'edit': edit,
        'duration': actual_duration, 'width': 1280, 'height': 800,
        'codec': 'h264', 'audio': False, 'subtitles': False,
        'bytes': video_path.stat().st_size, 'sha256': digest,
        'video': str(video_path), 'poster': str(poster_path),
        'contactSheet': str(qa / f'{item_id}-contact.jpg'),
    }
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('ids', nargs='*', help='Capture IDs; default: all ready captures')
    parser.add_argument('--legacy', action='store_true', help='Also import the reviewed six legacy captures')
    parser.add_argument('--input', type=Path, default=CAPTURES / 'passive')
    parser.add_argument('--output', type=Path, default=LANDING / 'public/media/demos')
    parser.add_argument('--qa', type=Path, default=CAPTURES / 'passive-qa')
    parser.add_argument('--jobs', type=int, default=2)
    parser.add_argument('--force', action='store_true')
    args = parser.parse_args()
    sources = {item_id: CAPTURES / filename for item_id, filename in LEGACY.items()} if args.legacy else {}
    # A newer ready capture always supersedes a legacy capture with the same ID.
    for marker in args.input.glob('*.ready'):
        if marker.stem in CATALOG_ORDER:
            sources[marker.stem] = marker.with_suffix('.mov')
    if args.ids:
        missing = set(args.ids) - sources.keys()
        if missing:
            parser.error(f'No ready capture for: {", ".join(sorted(missing))}')
        sources = {item_id: sources[item_id] for item_id in args.ids}
    for item_id, source in sources.items():
        if not item_id or any(ch not in 'abcdefghijklmnopqrstuvwxyz0123456789-' for ch in item_id):
            parser.error(f'Invalid capture ID: {item_id}')
        if not source.is_file():
            parser.error(f'Missing capture: {source}')
    args.output.mkdir(parents=True, exist_ok=True)
    args.qa.mkdir(parents=True, exist_ok=True)
    errors = 0
    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        pending = {pool.submit(encode, item_id, source, args.output, args.qa, args.force): item_id
                   for item_id, source in sources.items()}
        for task in as_completed(pending):
            try:
                result = task.result()
            except Exception as exc:
                result = {'id': pending[task], 'status': 'error', 'error': str(exc)}
            errors += result['status'] == 'error'
            print(json.dumps(result, ensure_ascii=False), flush=True)
    if args.output.resolve() == (LANDING / 'public/media/demos').resolve():
        recordings = []
        for item_id in CATALOG_ORDER:
            report_path = args.qa / f'{item_id}.json'
            if not report_path.exists() or not (args.output / f'{item_id}.mp4').exists():
                continue
            report = json.loads(report_path.read_text())
            recordings.append({
                'id': item_id, 'src': f'/media/demos/{item_id}.mp4',
                'poster': f'/media/demos/{item_id}.jpg',
                'sourceKind': 'native-screen-recording',
                'duration': report['duration'], 'width': report['width'], 'height': report['height'],
                'codec': report['codec'], 'audio': False, 'subtitles': False,
                'bytes': report['bytes'], 'sha256': report['sha256'],
                'edit': None if not report.get('edit') else {
                    'reason': report['edit']['reason'], 'segments': report['edit']['segments'],
                    'additionalSource': 'pdf-analyze-result.mov' if 'additionalSource' in report['edit'] else None,
                },
            })
        manifest = {
            'version': 3, 'recordingsInUse': True, 'expectedCount': len(CATALOG_ORDER),
            'recordingCount': len(recordings), 'playback': 'muted-autoplay-loop-no-controls',
            'sampleContent': '심리학의 이해 — 기억과 학습',
            'recordings': recordings,
            'supportingAssets': ['app-overview.jpg', 'pdf-overview.jpg', 'psychology-demo.pdf'],
        }
        manifest_path = LANDING / 'public/media/manifest.json'
        manifest_text = json.dumps(manifest, ensure_ascii=False, indent=2) + '\n'
        if not manifest_path.exists() or manifest_path.read_text() != manifest_text:
            temporary_manifest = manifest_path.with_suffix('.tmp')
            temporary_manifest.write_text(manifest_text)
            os.replace(temporary_manifest, manifest_path)
    return 1 if errors else 0


if __name__ == '__main__':
    raise SystemExit(main())
