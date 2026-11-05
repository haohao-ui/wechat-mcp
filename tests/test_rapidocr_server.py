"""Unit tests for the RapidOCR helper protocol; the real model is not needed."""
import importlib.util
import io
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('rapidocr_server', ROOT/'native'/'rapidocr_server.py')
server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(server)

BOX = [[1, 2], [11, 2], [11, 8], [1, 8]]


class HandleTests(unittest.TestCase):
    def test_lines_are_converted(self):
        engine = lambda path: ([[BOX, '你好', 0.98]], 0.1)
        self.assertEqual(server.handle({'image': '/x.png'}, engine),
                         {'lines': [{'text': '你好', 'box': [[1.0, 2.0], [11.0, 2.0], [11.0, 8.0], [1.0, 8.0]], 'score': 0.98}]})

    def test_blank_image_is_empty_not_error(self):
        self.assertEqual(server.handle({'image': '/x.png'}, lambda p: (None, 0.0)), {'lines': []})

    def test_bad_request_and_engine_error(self):
        self.assertIn('error', server.handle({}, lambda p: (None, 0)))
        self.assertIn('error', server.handle({'image': 3}, lambda p: (None, 0)))
        def boom(path):
            raise RuntimeError('model')
        self.assertEqual(server.handle({'image': '/x.png'}, boom), {'error': 'RuntimeError: model'})

    def test_text_with_control_characters_round_trips(self):
        engine = lambda path: ([[BOX, 'a\tb\nc', '0.5']], 0.1)
        line = json.dumps(server.handle({'image': '/x.png'}, engine), ensure_ascii=False)
        self.assertEqual(json.loads(line)['lines'][0]['text'], 'a\tb\nc')
        self.assertEqual(json.loads(line)['lines'][0]['score'], 0.5)

    def test_serve_loop_writes_ready_then_one_line_per_request(self):
        stdin = io.StringIO('{"image":"/a.png"}\n\nnot json\n{"image":"/b.png"}\n')
        stdout = io.StringIO()
        server.serve(stdin, stdout, lambda path: ([[BOX, path, 1]], 0))
        lines = [json.loads(l) for l in stdout.getvalue().splitlines()]
        self.assertEqual(lines[0], {'ready': True})
        self.assertEqual(lines[1]['lines'][0]['text'], '/a.png')
        self.assertIn('error', lines[2])
        self.assertEqual(lines[3]['lines'][0]['text'], '/b.png')
        self.assertEqual(len(lines), 4)


if __name__ == '__main__':
    unittest.main()
