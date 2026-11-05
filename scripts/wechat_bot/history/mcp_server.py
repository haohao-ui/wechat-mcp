"""Read-only stdio MCP server over the conversation history (design §10.3)."""
import argparse
import base64
import json
from pathlib import Path
import sys

from .query import HistoryQuery, QueryError

S, I = {'type': 'string'}, {'type': 'integer'}
PAGE = {'cursor': S, 'since': I, 'until': I}
TOOLS = {
    'history_list_contacts': ('list_contacts', '列出联系人（当前名称、全部别名、消息数、最后消息时间）',
                              {'query': S, 'cursor': S, 'limit': I}, []),
    'history_get_contact': ('get_contact', '按 contact_id 或名称（表情/符号不敏感）获取联系人',
                            {'contact_id': S, 'name': S}, []),
    'history_get_recent_messages': ('get_recent_messages', '某联系人最近 N 条消息（旧→新）',
                                    {'contact_id': S, 'limit': I}, ['contact_id']),
    'history_get_history': ('get_history', '某联系人完整历史，按时间分页',
                            {'contact_id': S, **PAGE, 'limit': I, 'order': {'type': 'string', 'enum': ['asc', 'desc']}}, ['contact_id']),
    'history_search_messages': ('search_messages', '全文搜索消息（含图片/文件的模型理解要点），返回命中消息及上下文消息 ID',
                                {'q': S, 'contact_id': S, **PAGE, 'limit': I}, ['q']),
    'history_get_message': ('get_message', '单条消息详情（含 meta；附件含来源 original/chat_thumbnail 与理解要点 understanding）', {'message_id': S}, ['message_id']),
    'history_get_media': ('get_media', '附件元数据（来源、理解要点 understanding）与 base64 内容（≤10MB）', {'media_id': S}, ['media_id']),
}


def tool_list():
    return [{'name': name, 'description': desc,
             'inputSchema': {'type': 'object', 'properties': props, 'required': required, 'additionalProperties': False}}
            for name, (_, desc, props, required) in TOOLS.items()]


def call_tool(query, name, arguments):
    if name not in TOOLS:
        raise QueryError('invalid_argument', 'unknown tool')
    method, _, props, required = TOOLS[name]
    if not isinstance(arguments, dict) or set(arguments) - set(props) or any(k not in arguments for k in required):
        raise QueryError('invalid_argument', 'invalid arguments')
    for key, value in arguments.items():
        expected = int if props[key]['type'] == 'integer' else str
        if type(value) is not expected:
            raise QueryError('invalid_argument', f'{key} must be {props[key]["type"]}')
    result = getattr(query, method)(**arguments)
    if method == 'get_media':
        info, path = result
        result = {'media': info, 'base64': base64.b64encode(path.read_bytes()).decode()}
    return result


def handle(query, request):
    method, rid = request.get('method'), request.get('id')
    if rid is None:
        return None  # notification
    if method == 'initialize':
        result = {'protocolVersion': '2024-11-05', 'capabilities': {'tools': {}},
                  'serverInfo': {'name': 'wechat-history', 'version': '1'}}
    elif method == 'tools/list':
        result = {'tools': tool_list()}
    elif method == 'tools/call':
        params = request.get('params') or {}
        try:
            value = call_tool(query, params.get('name'), params.get('arguments', {}))
            result = {'content': [{'type': 'text', 'text': json.dumps(value, ensure_ascii=False)}], 'isError': False}
        except QueryError as error:
            result = {'content': [{'type': 'text', 'text': json.dumps({'error': {'code': error.code, 'message': error.message}}, ensure_ascii=False)}],
                      'isError': True}
    elif method == 'ping':
        result = {}
    else:
        return {'jsonrpc': '2.0', 'id': rid, 'error': {'code': -32601, 'message': 'method not found'}}
    return {'jsonrpc': '2.0', 'id': rid, 'result': result}


def open_query(config_path):
    from scripts.wechat_bot.service import load_config
    config = load_config(config_path)
    return HistoryQuery(Path(config['state_dir'])/'history'/'history.sqlite3', default_recent=config['context_messages'] or 5), config


def default_config():
    return Path.home()/'.WeChatRPABot'/'config.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', default=str(default_config()))
    query, _ = open_query(parser.parse_args().config)
    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            response = {'jsonrpc': '2.0', 'id': None, 'error': {'code': -32700, 'message': 'parse error'}}
        else:
            response = handle(query, request)
        if response is not None:
            sys.stdout.write(json.dumps(response, ensure_ascii=False) + '\n')
            sys.stdout.flush()


if __name__ == '__main__':
    main()
