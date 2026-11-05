#!/usr/bin/env python3
"""RapidOCR helper for the WeChat desktop worker: one JSON object per line.

Persistent:  rapidocr_server.py          reads {"image": "<png path>"} lines from stdin
Single shot: rapidocr_server.py --once <png path>
Both first print {"ready": true} once the model is loaded, then
{"lines": [{"text", "box": [[x, y] x4], "score"}]} or {"error": "..."} per image.
Runs fully offline; the models ship inside the rapidocr_onnxruntime package.
Keep this file Python 3.9 compatible (macOS 12 system python).
"""
import json
import sys


def handle(request, engine):
    image = request.get('image') if isinstance(request, dict) else None
    if not isinstance(image, str) or not image:
        return {'error': 'invalid_request'}
    try:
        result, _ = engine(image)
    except Exception as error:  # report, keep serving
        return {'error': f'{type(error).__name__}: {error}'}
    lines = []
    for box, text, score in result or []:
        lines.append({'text': str(text),
                      'box': [[float(x), float(y)] for x, y in box],
                      'score': float(score)})
    return {'lines': lines}


def emit(stdout, value):
    stdout.write(json.dumps(value, ensure_ascii=False) + '\n')
    stdout.flush()


def serve(stdin, stdout, engine):
    emit(stdout, {'ready': True})
    for raw in stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            request = json.loads(raw)
        except ValueError:
            emit(stdout, {'error': 'invalid_json'})
            continue
        emit(stdout, handle(request, engine))


def main(argv):
    # Library warnings must not reach stdout, which carries the protocol.
    real_stdout, sys.stdout = sys.stdout, sys.stderr
    try:
        from rapidocr_onnxruntime import RapidOCR
        engine = RapidOCR()
    except Exception as error:
        emit(real_stdout, {'error': f'rapidocr_unavailable: {type(error).__name__}: {error}'})
        return 2
    if len(argv) == 3 and argv[1] == '--once':
        emit(real_stdout, {'ready': True})
        emit(real_stdout, handle({'image': argv[2]}, engine))
        return 0
    if len(argv) != 1:
        emit(real_stdout, {'error': 'usage: rapidocr_server.py [--once <image>]'})
        return 2
    serve(sys.stdin, real_stdout, engine)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
