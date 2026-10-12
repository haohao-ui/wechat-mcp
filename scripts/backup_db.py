"""Create a consistent SQLite backup without copying an active WAL file."""
import argparse
from contextlib import closing
import os
from pathlib import Path
import sqlite3
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument('--source', required=True, type=Path)
parser.add_argument('--output', required=True, type=Path)
args = parser.parse_args()
source = args.source.resolve(strict=True)
output = args.output.absolute()
if output.exists():
    parser.error('Output already exists; choose a new backup path.')
with tempfile.NamedTemporaryFile(dir=output.parent, prefix='.service-backup-', delete=False) as handle:
    temporary = Path(handle.name)
try:
    # WAL databases may need writable sidecars even when only reading.
    # mode=rw permits that initialization while refusing to create a missing source.
    with closing(sqlite3.connect(source.as_uri()+'?mode=rw', uri=True)) as reader:
        with closing(sqlite3.connect(temporary)) as writer:
            reader.backup(writer)
            if writer.execute('PRAGMA integrity_check').fetchone()[0] != 'ok':
                raise RuntimeError('Backup integrity check failed')
    os.link(temporary,output)
    print('Backup created and integrity verified:',output)
finally:
    temporary.unlink(missing_ok=True)
