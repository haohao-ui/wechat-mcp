import hashlib
import json
import unicodedata


class AmbiguousAnchor(ValueError):
    """Visible history overlaps, but identical bubbles make the overlap position unknowable.

    `certain` holds the bubbles after the latest plausible overlap: new under every interpretation.
    """

    def __init__(self, message, certain):
        super().__init__(message)
        self.certain = certain


class ContinuityGap(ValueError):
    """No overlap with the cursor at all (e.g. history cleared or long absence)."""


def certain_tail(old, new, current):
    end = max((i + k for k in range(1, min(len(old), len(new)) + 1)
               for i in range(len(new) - k + 1) if new[i:i+k] == old[-k:]), default=len(new))
    return current[end:]


# Image digests encode only the bubble size; `phash` (64-bit dHash, hex) tells same-size images apart
# and survives WeChat's blurred-preview -> loaded transition within PHASH_TOLERANCE bits.
PHASH_TOLERANCE = 12
FINGERPRINT_VERSION = 2


def fingerprint(bubble):
    return (bubble['direction'], bubble['digest'], bubble.get('phash'))


def phash_distance(a, b):
    return bin(int(a, 16) ^ int(b, 16)).count('1')


def reconcile(previous, current):
    """Copy of `current` where each image matching a previous one (same direction and size digest,
    phash within tolerance) takes that image's identity, so loading never looks like a new message."""
    result = [dict(b) for b in current]
    used = set()
    for item in result:
        if not item.get('phash'):
            continue
        for index, old in enumerate(previous):
            if (index in used or not old.get('phash') or old['direction'] != item['direction']
                    or old['digest'] != item['digest']):
                continue
            if phash_distance(old['phash'], item['phash']) <= PHASH_TOLERANCE:
                used.add(index)
                # media_id stays current: the newest pixels are the loaded image, not the blurred preview.
                item['phash'] = old['phash']
                break
    return result


def new_bubbles(previous, current):
    old, new = list(map(fingerprint, previous)), list(map(fingerprint, current))
    if old == new:
        return []
    if not old:
        return current
    matches = [k for k in range(1, min(len(old), len(new)) + 1) if old[-k:] == new[:k]]
    if not matches:
        raise ContinuityGap('visual continuity gap or ambiguous repeated anchor')
    size = max(matches)
    anchor = old[-size:]
    # A repeated short greeting must not invalidate a unique, longer conversation context.
    # Uniform/periodic sequences still cannot identify which occurrence was retained by scrolling.
    periodic = any(size % period == 0 and anchor == anchor[:period] * (size // period)
                   for period in range(1, size))
    if len(matches) > 1 and (len(set(anchor)) < 2 or periodic):
        raise AmbiguousAnchor('ambiguous repeated anchor', certain_tail(old, new, current))
    if sum(new[i:i+size] == anchor for i in range(len(new)-size+1)) != 1:
        raise AmbiguousAnchor('repeated visual anchor', certain_tail(old, new, current))
    return current[size:]


def stitch(cursor, pages):
    """New bubbles after `cursor`, joined across scrolled pages ordered oldest -> newest (last = current).

    The oldest page overlapping the cursor starts the run; each later page must overlap its predecessor.
    """
    for index, page in enumerate(pages):
        try:
            changes = new_bubbles(cursor, page)
        except ContinuityGap:
            continue
        for older, newer in zip(pages[index:], pages[index+1:]):
            changes = changes + new_bubbles(older, newer)
        return changes
    raise ContinuityGap('no scrolled page overlaps the cursor')


class Policy:
    def __init__(self, account, recipient, trigger, names=None):
        if not account or not recipient or not trigger:
            raise ValueError('explicit account, exact private-chat allowlist and test trigger required')
        self.account, self.recipient, self.trigger = account, recipient, trigger
        from .history.names import clean_title
        self.clean = clean_title
        # Exact title match against the contact's configured name, aliases and confirmed OCR spellings.
        self.accepted = {clean_title(n) for n in (names or [recipient])}

    def check_frame(self, frame):
        if frame.get('profile') not in ('wechat-4-dark', 'wechat-4-light') or frame.get('scale') not in (1, 2):
            raise ValueError('uncalibrated visual profile')
        if frame.get('clipped_bubble'):
            raise ValueError('incomplete bottom bubble; no automatic reply')
        titles = [self.clean(x) for x in frame.get('title_lines', [])]
        if len(titles) != 1 or titles[0] not in self.accepted:
            raise ValueError('exact allowlisted title not verified')

    def candidate(self, previous, current, confirmed):
        self.check_frame(current)
        self.check_frame(confirmed)
        changes = new_bubbles(previous['bubbles'], current['bubbles'])
        if list(map(fingerprint, current['bubbles'])) != list(map(fingerprint, confirmed['bubbles'])):
            raise ValueError('chat changed during confirmation')
        incoming = [x for x in changes if x['direction'] == 'inbound_visual']
        if len(incoming) != 1 or len(changes) != 1:
            return None
        message = incoming[0]
        if not ''.join(message['text'].split()).startswith(''.join(self.trigger.split())):
            return None
        return message

    def key(self, baseline, candidate):
        identity = [self.account, self.recipient, list(map(fingerprint, baseline['bubbles'])), fingerprint(candidate)]
        return 'bot-' + hashlib.sha256(json.dumps(identity, ensure_ascii=False).encode()).hexdigest()


def validate_reply(value):
    if not isinstance(value, str) or not value.strip() or len(value) > 200:
        raise ValueError('model reply must be 1..200 characters')
    if any(ord(x) < 32 and x != '\n' for x in value):
        raise ValueError('reply contains control characters')
    return value


def echo_matches(observed, expected):
    compact = lambda value: ''.join(unicodedata.normalize('NFKC', value).split())
    return compact(observed) == compact(expected)
