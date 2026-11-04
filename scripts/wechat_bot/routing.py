"""Conversation rules. Exclusions take precedence over explicitly allowed private chats.

Names compare by `name_key`, so OCR variants of emoji/symbol names resolve to the same entry."""
from .history.names import candidate_names, name_key


def validate_rules(config):
    keys={}
    for name in ('allowlist', 'ignorelist'):
        entries=config.get(name, [])
        if not isinstance(entries,list) or len(entries)>200:
            raise ValueError(name+' must be an array of at most 200 conversations')
        if name=='allowlist' and not entries:
            raise ValueError('at least one explicitly allowlisted private contact required')
        seen=set()
        for entry in entries:
            if not isinstance(entry,dict) or not {'recipient','kind'}<=set(entry)<={'recipient','kind','aliases'}:
                raise ValueError('invalid '+name+' entry')
            recipient=entry['recipient'];kind=entry['kind']
            aliases=entry.get('aliases',[])
            if not isinstance(aliases,list) or len(aliases)>10:
                raise ValueError('aliases must be an array of at most 10 names')
            for value in [recipient,*aliases]:
                if not isinstance(value,str) or not value.strip() or value!=value.strip():
                    raise ValueError('exact conversation name without surrounding whitespace required')
            if kind not in ('private','group') or (name=='allowlist' and kind!='private'):
                raise ValueError('only private chats can be allowlisted; exclusions support private/group')
            identity=(recipient,kind)
            if identity in seen:
                raise ValueError('duplicate '+name+' entry')
            seen.add(identity)
            if name=='allowlist':
                for value in candidate_names(entry):
                    key=name_key(value)
                    if keys.setdefault(key,id(entry))!=id(entry):
                        raise ValueError('allowlist names collide after normalization')


def match_entry(config, raw_name, kind='private'):
    key=name_key(raw_name)
    for entry in config.get('allowlist', []):
        if entry['kind']==kind and any(name_key(n)==key for n in candidate_names(entry)):
            return entry
    return None


def decision(config, recipient, kind='private'):
    # The desktop cannot prove chat kind/WeChat ID. A same-title exclusion of either kind wins.
    key=name_key(recipient)
    if any(name_key(n)==key for x in config.get('ignorelist', []) for n in candidate_names(x)):
        return 'ignored'
    if match_entry(config,recipient,kind) is None:
        return 'not_allowlisted'
    return 'allowed'
