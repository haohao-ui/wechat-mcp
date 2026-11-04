"""Contact-name normalization. OCR drops or alters emoji/symbols, so matching uses a stable key."""
import unicodedata

INVISIBLE = {chr(c) for c in range(0x200B, 0x2010)} | {'⁠', '﻿'}
VARIATION = {chr(c) for c in range(0xFE00, 0xFE10)}


def clean_title(raw):
    """Trim whitespace and zero-width characters only; the native worker applies the same rule."""
    return ''.join(c for c in raw if c not in INVISIBLE).strip()


def name_key(raw):
    text = unicodedata.normalize('NFKC', raw)
    visible = ''.join(c for c in text if c not in INVISIBLE and c not in VARIATION and not c.isspace())
    key = ''.join(c for c in visible if unicodedata.category(c) not in ('So', 'Sk', 'Cf'))
    return key or visible


def candidate_names(entry):
    return [entry['recipient'], *entry.get('aliases', [])]
