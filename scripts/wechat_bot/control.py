"""Install and inspect the current user's launchd bot service. Never edits the remote GUI bundle."""
import argparse
import base64
import json
import os
from pathlib import Path
import plistlib
import re
import shlex
import shutil
import subprocess
import sys
import time

from scripts.agent_bridge.codex import private_directory, write_private
from .service import load_config

LABEL='local.wechat-rpa.bot'


def locations():
    base=Path.home()/'.WeChatRPABot'
    return base, base/'config.json', Path.home()/'Library/LaunchAgents'/f'{LABEL}.plist'


def loaded():
    return subprocess.run(['launchctl','print',f'gui/{os.getuid()}/{LABEL}'],
                          stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0


HISTORY_LABEL='local.wechat-rpa.history'


def history_http_plist(config_path, logs):
    return {'Label':HISTORY_LABEL,
            'ProgramArguments':[sys.executable,'-u','-m','scripts.wechat_bot.history.http_server','--config',str(config_path)],
            'WorkingDirectory':str(Path(__file__).resolve().parents[2]),
            'RunAtLoad':True,'KeepAlive':True,'ThrottleInterval':15,'ProcessType':'Background',
            'StandardOutPath':str(Path(logs)/'history-http.stdout.log'),'StandardErrorPath':str(Path(logs)/'history-http.stderr.log')}


def history_http(action, config_path, base):
    plist_path=Path.home()/'Library/LaunchAgents'/f'{HISTORY_LABEL}.plist'
    domain=f'gui/{os.getuid()}'
    is_loaded=lambda: subprocess.run(['launchctl','print',f'{domain}/{HISTORY_LABEL}'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode==0
    if action=='install':
        config=load_config(config_path)
        logs=private_directory(base/'logs')
        for name in ['history-http.stdout.log','history-http.stderr.log']:
            (logs/name).touch(mode=0o600,exist_ok=True)
        plist_path.parent.mkdir(parents=True,exist_ok=True)
        with open(plist_path,'wb') as handle: plistlib.dump(history_http_plist(config_path,logs),handle)
        plist_path.chmod(0o600)
        print(json.dumps({'status':'installed','label':HISTORY_LABEL,'bind':config['history_http']['bind'],'port':config['history_http']['port']}))
    elif action=='start':
        subprocess.run(['launchctl','enable',f'{domain}/{HISTORY_LABEL}'],check=True)
        if not is_loaded(): subprocess.run(['launchctl','bootstrap',domain,str(plist_path)],check=True)
        print(json.dumps({'status':'started','label':HISTORY_LABEL}))
    elif action=='stop':
        subprocess.run(['launchctl','disable',f'{domain}/{HISTORY_LABEL}'],check=True)
        if is_loaded(): subprocess.run(['launchctl','bootout',f'{domain}/{HISTORY_LABEL}'],check=True)
        print(json.dumps({'status':'stopped','label':HISTORY_LABEL}))
    else:
        config=load_config(config_path)
        print(json.dumps({'launchd_loaded':is_loaded(),'bind':config['history_http']['bind'],'port':config['history_http']['port'],
                          'token_file':str(Path(config['state_dir'])/'history'/'http-token')}))


def history_command(args):
    from .history.migrate import migrate
    from .history.store import HistoryStore
    config=load_config(args.config)
    if args.action=='migrate-history':
        print(json.dumps(migrate(config),ensure_ascii=False)); return
    if args.action=='history-backup':
        target=Path(args.to)
        if target.exists():
            raise SystemExit('backup target already exists: '+str(target))
    store=HistoryStore.open(config['state_dir'])
    try:
        def contact(value):
            found=value if value.startswith('c_') else store.resolve(config['account_id'],value)
            if not found or store.conn.execute('SELECT 1 FROM contacts WHERE contact_id=?',(found,)).fetchone() is None:
                raise SystemExit('unknown contact: '+value)
            return found
        if args.action=='merge-contact':
            print(json.dumps({'merge_id':store.merge(contact(args.source),contact(args.into),args.note or '')}))
        elif args.action=='unmerge-contact':
            store.unmerge(args.merge_id); print(json.dumps({'unmerged':args.merge_id}))
        elif args.action=='history-backup':
            descriptor=os.open(target,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600); os.close(descriptor)
            import sqlite3
            destination=sqlite3.connect(target)
            try: store.conn.backup(destination)
            finally: destination.close()
            target.chmod(0o600)
            print(json.dumps({'backup':str(target),'bytes':target.stat().st_size}))
        else:
            db=store.root/'history.sqlite3'
            contacts=[]
            for row in store.conn.execute('SELECT contact_id, display_name, merged_into FROM contacts ORDER BY rowid').fetchall():
                count,last=store.conn.execute('SELECT COUNT(*), MAX(observed_at) FROM messages WHERE contact_id=?',(row[0],)).fetchone()
                unmatched=[r[0] for r in store.conn.execute(
                    "SELECT raw_name FROM contact_names o WHERE contact_id=? AND source='observed' AND NOT EXISTS "
                    "(SELECT 1 FROM contact_names c WHERE c.contact_id=o.contact_id AND c.name_key=o.name_key AND c.source IN ('config','alias'))",(row[0],))]
                contacts.append({'contact_id':row[0],'display_name':row[1],'merged_into':row[2],'messages':count,
                                 'last_observed_at':last,'unmatched_observed_names':unmatched})
            # Merged contacts: their names should be added to the target's allowlist aliases.
            suggestions=[]
            for row in store.conn.execute('SELECT contact_id FROM contacts WHERE merged_into IS NOT NULL ORDER BY rowid').fetchall():
                into=store.canonical(row[0])
                names=[r[0] for r in store.conn.execute("SELECT raw_name FROM contact_names WHERE contact_id=? AND source IN ('config','alias') ORDER BY id",(row[0],))]
                into_name=store.conn.execute('SELECT display_name FROM contacts WHERE contact_id=?',(into,)).fetchone()[0]
                suggestions.append({'into':into,'into_name':into_name,'add_aliases':names})
            print(json.dumps({'database':str(db),'bytes':db.stat().st_size,'alias_suggestions':suggestions,
                              'messages':store.conn.execute('SELECT COUNT(*) FROM messages').fetchone()[0],
                              'pending_file_downloads':store.conn.execute("SELECT COUNT(*) FROM media_jobs WHERE status='pending'").fetchone()[0],
                              'contacts':contacts},ensure_ascii=False))
    finally:
        store.conn.close()


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='action',required=True)
    default_config=str(locations()[1])
    for action in ['migrate-history','history-status']:
        sub.add_parser(action).add_argument('--config',default=default_config)
    merge=sub.add_parser('merge-contact')
    merge.add_argument('--from',dest='source',required=True); merge.add_argument('--into',required=True)
    merge.add_argument('--note'); merge.add_argument('--config',default=default_config)
    unmerge=sub.add_parser('unmerge-contact')
    unmerge.add_argument('--merge-id',type=int,required=True); unmerge.add_argument('--config',default=default_config)
    backup=sub.add_parser('history-backup')
    backup.add_argument('--to',required=True); backup.add_argument('--config',default=default_config)
    install=sub.add_parser('install')
    install.add_argument('--host',required=True)
    install.add_argument('--account',required=True)
    install.add_argument('--recipient',required=True)
    install.add_argument('--allow-replies',action='store_true',required=True)
    for action in ['status','start','stop']: sub.add_parser(action)
    cal = sub.add_parser('calibrate')
    cal.add_argument('--accept', action='store_true', help='Accept candidate layout after validation')
    cal.add_argument('--config', default=default_config)
    cal.add_argument('--layout-dir', default=None, help='Layout directory override')
    sub.add_parser('history-http').add_argument('command',choices=['install','start','stop','status'])
    args=parser.parse_args(argv)
    base,config_path,plist_path=locations()
    if args.action in ('migrate-history','history-status','merge-contact','unmerge-contact','history-backup'):
        return history_command(args)
    if args.action=='history-http':
        return history_http(args.command,config_path,base)
    if args.action=='calibrate':
        return calibrate_command(args)
    if args.action=='install':
        if loaded() or config_path.exists():
            raise RuntimeError('bot service already configured; inspect status and config before changing it')
        if not shutil.which('codex'):
            raise RuntimeError('Codex CLI required')
        private_directory(base)
        logs=private_directory(base/'logs')
        config={'schema_version':1,'host':args.host,'account_id':args.account,
                'allowlist':[{'recipient':args.recipient,'kind':'private'}],'ignorelist':[],
                'state_dir':str(base/'state'),'poll_seconds':2,'cooldown_seconds':15,'max_replies_per_hour':20}
        write_private(config_path,config)
        load_config(config_path)
        log_paths=[logs/'stdout.log',logs/'stderr.log']
        for path in log_paths:
            path.touch(mode=0o600,exist_ok=True); path.chmod(0o600)
        plist_path.parent.mkdir(parents=True,exist_ok=True)
        value={'Label':LABEL,'ProgramArguments':[sys.executable,'-u','-m','scripts.wechat_bot.service','--config',str(config_path)],
               'WorkingDirectory':str(Path(__file__).resolve().parents[2]),
               'RunAtLoad':True,'KeepAlive':True,'ThrottleInterval':15,'ExitTimeOut':240,'ProcessType':'Background',
               'StandardOutPath':str(log_paths[0]),'StandardErrorPath':str(log_paths[1]),
               'EnvironmentVariables':{'PATH':str(Path(shutil.which('codex')).parent)+':/usr/local/bin:/usr/bin:/bin'}}
        with open(plist_path,'wb') as handle: plistlib.dump(value,handle)
        plist_path.chmod(0o600)
        subprocess.run(['launchctl','enable',f'gui/{os.getuid()}/{LABEL}'],check=True)
        subprocess.run(['launchctl','bootstrap',f'gui/{os.getuid()}',str(plist_path)],check=True)
        print(json.dumps({'status':'installed','label':LABEL,'config':str(config_path),'allowlist':[args.recipient]},ensure_ascii=False))
    elif args.action=='start':
        subprocess.run(['launchctl','enable',f'gui/{os.getuid()}/{LABEL}'],check=True)
        if not loaded():
            load_config(config_path)
            subprocess.run(['launchctl','bootstrap',f'gui/{os.getuid()}',str(plist_path)],check=True)
        print(json.dumps({'status':'started','label':LABEL}))
    elif args.action=='stop':
        subprocess.run(['launchctl','disable',f'gui/{os.getuid()}/{LABEL}'],check=True)
        if loaded(): subprocess.run(['launchctl','bootout',f'gui/{os.getuid()}/{LABEL}'],check=True)
        deadline=time.monotonic()+20
        while loaded() and time.monotonic()<deadline:
            time.sleep(0.1)
        if loaded():
            raise RuntimeError('service is still unloading; do not start a replacement yet')
        print(json.dumps({'status':'stopped','label':LABEL}))
    else:
        state_dir=Path(load_config(config_path)['state_dir']) if config_path.exists() else base/'state'
        health_path=state_dir/'health.json'
        health=json.loads(health_path.read_text()) if health_path.exists() else None
        journal_root=state_dir/'sessions' if (state_dir/'sessions').exists() else state_dir
        records=[json.loads(p.read_text()) for p in journal_root.rglob('send-*.json')]
        stats={'recorded_batches':len(records),'triggered':sum(r.get('status')=='triggered' for r in records),
               'client_echo_observed':sum(r.get('client_echo_observed') is True for r in records),
               'needs_review':sum(r.get('status') in ['generating','generated','attempting','unknown'] for r in records)}
        needs_recalibration=check_needs_recalibration(health,state_dir)
        if needs_recalibration:
            sys.stderr.write('界面核对连续失败，请运行 control calibrate 重新定位\n')
            sys.stderr.flush()
        print(json.dumps({'launchd_loaded':loaded(),'health':health,'stats':stats,'config':str(config_path),'needs_recalibration':needs_recalibration},ensure_ascii=False))


def check_needs_recalibration(health, state_dir=None):
    if health and isinstance(health, dict):
        if health.get('state') == 'needs_recalibration':
            return True
        sessions = health.get('sessions')
        if isinstance(sessions, dict):
            for s in sessions.values():
                if isinstance(s, dict) and s.get('state') == 'needs_recalibration':
                    return True
    if state_dir:
        sessions_dir = Path(state_dir) / 'sessions'
        if sessions_dir.exists():
            for sp in sessions_dir.glob('*/health.json'):
                try:
                    s_data = json.loads(sp.read_text())
                    if isinstance(s_data, dict) and s_data.get('state') == 'needs_recalibration':
                        return True
                except Exception:
                    pass
    return False


VERSION_RE = re.compile(r'^[0-9]+(\.[0-9]+){1,3}$')


def validate_version_string(version):
    if not isinstance(version, str):
        return False
    return bool(VERSION_RE.match(version.strip()))


# Relative to the remote home (ssh starts there); a quoted "~" would not expand.
REMOTE_LAYOUT_DIR = '.wechat-rpa/layout'
# Read-only readiness probe name: the session list returns no rows for it.
CALIBRATE_PROBE = 'calibrate-probe'


def check_remote(proc, what):
    if proc.returncode != 0:
        raise RuntimeError(f'remote write failed ({what}): ' + (proc.stderr or b'').decode('utf-8', 'replace')[:200])


def default_remote_runner(host, command, input_data=None):
    cmd = ['ssh', '-o', 'BatchMode=yes', '-o', 'LogLevel=ERROR', host, command]
    return subprocess.run(cmd, input=input_data, capture_output=True, check=True)


def validate_layout_candidate(candidate):
    if not isinstance(candidate, dict):
        raise ValueError('layout_unreadable')
    if candidate.get('schema') != 1:
        raise ValueError('layout_schema_unsupported')
    window = candidate.get('window')
    if not isinstance(window, dict) or 'width' not in window or 'height' not in window:
        raise ValueError('layout_invalid_value:window')
    try:
        win_w = float(window['width'])
        win_h = float(window['height'])
    except (ValueError, TypeError):
        raise ValueError('layout_invalid_value:window')
    regions = candidate.get('regions')
    if not isinstance(regions, dict):
        raise ValueError('layout_unreadable')
    required_regions = ['session_list', 'title', 'chat', 'input']
    for req in required_regions:
        if req not in regions:
            raise ValueError(f'layout_missing_region:{req}')
    for name, rect in regions.items():
        if not isinstance(rect, dict):
            raise ValueError(f'layout_invalid_value:{name}')
        try:
            x = float(rect.get('x', 0))
            y = float(rect.get('y', 0))
            w = float(rect.get('w', 0))
            h = float(rect.get('h', 0))
        except (ValueError, TypeError):
            raise ValueError(f'layout_invalid_value:{name}')
        if w <= 0:
            raise ValueError(f'layout_invalid_value:{name}.w')
        if h <= 0:
            raise ValueError(f'layout_invalid_value:{name}.h')
        if x < 0 or y < 0 or (x + w) > win_w or (y + h) > win_h:
            raise ValueError(f'layout_region_out_of_window:{name}')
    scroll = candidate.get('scroll')
    if not isinstance(scroll, dict):
        raise ValueError('layout_invalid_value:scroll')
    try:
        if float(scroll.get('points_per_tick', 0)) <= 0:
            raise ValueError('layout_invalid_value:scroll.points_per_tick')
        if int(scroll.get('lines_per_tick', 0)) <= 0:
            raise ValueError('layout_invalid_value:scroll.lines_per_tick')
    except (ValueError, TypeError):
        raise ValueError('layout_invalid_value:scroll')


def derive_candidate_fields(base_layout, measured):
    candidate = json.loads(json.dumps(base_layout))
    window_h = float(candidate.get('window', {}).get('height', 640.0))

    diffs = []
    missing = []

    expected_anchors = [a['name'] for a in candidate.get('anchors', [])]
    for name in expected_anchors:
        if name not in measured or measured[name] is None:
            missing.append(name)

    if 'input_divider' in measured and measured['input_divider'] is not None:
        divider = float(measured['input_divider'])
        if 'chat' in candidate.get('regions', {}):
            chat = candidate['regions']['chat']
            new_chat_h = divider - float(chat['y'])
            if chat.get('h') != new_chat_h:
                diffs.append({'field': 'chat.h', 'old': chat.get('h'), 'new': new_chat_h})
                chat['h'] = new_chat_h
        if 'chat_content' in candidate.get('regions', {}):
            chat_content = candidate['regions']['chat_content']
            new_cc_h = divider - float(chat_content['y'])
            if chat_content.get('h') != new_cc_h:
                diffs.append({'field': 'chat_content.h', 'old': chat_content.get('h'), 'new': new_cc_h})
                chat_content['h'] = new_cc_h
        if 'input' in candidate.get('regions', {}):
            inp = candidate['regions']['input']
            if inp.get('y') != divider:
                diffs.append({'field': 'input.y', 'old': inp.get('y'), 'new': divider})
                inp['y'] = divider
            new_inp_h = window_h - divider
            if inp.get('h') != new_inp_h:
                diffs.append({'field': 'input.h', 'old': inp.get('h'), 'new': new_inp_h})
                inp['h'] = new_inp_h

    if 'list_border' in measured and measured['list_border'] is not None:
        border = float(measured['list_border'])
        if 'session_list' in candidate.get('regions', {}):
            s_list = candidate['regions']['session_list']
            new_w = border - float(s_list['x'])
            if s_list.get('w') != new_w:
                diffs.append({'field': 'session_list.w', 'old': s_list.get('w'), 'new': new_w})
                s_list['w'] = new_w

    return diffs, missing, candidate


def require_host(config, host=None):
    host = host or (config or {}).get('host')
    if not host:
        raise ValueError('config has no WeChat host; calibrate never guesses one')
    return host


def run_worker_calibrate(config=None, host=None):
    host = require_host(config, host)
    from .native_capture import NativeCapture
    import asyncio
    async def _run():
        capture = NativeCapture(host, None, CALIBRATE_PROBE, Path.home() / '.wechat-rpa', '0'*64, names=[CALIBRATE_PROBE])
        await capture.connect()
        try:
            return await capture.calibrate()
        finally:
            await capture.close_transport()
    return asyncio.run(_run())


def run_calibrate(config=None, worker_result=None, host=None, layout_dir=None, remote_runner=None, worker_func=None):
    if remote_runner is None:
        remote_runner = default_remote_runner
    if layout_dir is None:
        layout_dir = REMOTE_LAYOUT_DIR
    if host is None:
        host = require_host(config)

    if worker_result is None:
        if worker_func:
            worker_result = worker_func()
        else:
            worker_result = run_worker_calibrate(config=config, host=host)

    if not isinstance(worker_result, dict):
        raise ValueError('layout_missing')

    version = worker_result.get('wechat_version')
    if not validate_version_string(version):
        raise ValueError('layout_missing')

    base_layout = worker_result.get('layout')
    if not isinstance(base_layout, dict):
        raise ValueError('layout_missing')

    measured = worker_result.get('measured', {})
    diffs, missing, candidate = derive_candidate_fields(base_layout, measured)
    candidate['wechat_version'] = version

    cand_json_path = f"{layout_dir}/candidate-wechat-{version}.json"
    cand_png_path = f"{layout_dir}/candidate-wechat-{version}.png"

    json_bytes = json.dumps(candidate, indent=2, ensure_ascii=False).encode('utf-8') + b'\n'
    cmd_json = f"umask 077 && mkdir -p {shlex.quote(layout_dir)} && cat > {shlex.quote(cand_json_path)} && chmod 600 {shlex.quote(cand_json_path)}"
    check_remote(remote_runner(host, cmd_json, input_data=json_bytes), 'candidate json')

    if 'annotated_png_base64' in worker_result and worker_result['annotated_png_base64']:
        raw_png = base64.b64decode(worker_result['annotated_png_base64'])
        cmd_png = f"umask 077 && mkdir -p {shlex.quote(layout_dir)} && cat > {shlex.quote(cand_png_path)} && chmod 600 {shlex.quote(cand_png_path)}"
        check_remote(remote_runner(host, cmd_png, input_data=raw_png), 'candidate png')

    return diffs, missing, candidate


def calibrate_accept(host=None, layout_dir=None, config=None, worker_func=None, remote_runner=None):
    if remote_runner is None:
        remote_runner = default_remote_runner
    if layout_dir is None:
        layout_dir = REMOTE_LAYOUT_DIR
    if host is None:
        host = require_host(config)

    # 1. 调 worker calibrate 获取当前运行状态
    if worker_func:
        worker_res = worker_func()
    else:
        worker_res = run_worker_calibrate(config=config, host=host)

    if not isinstance(worker_res, dict):
        raise ValueError('layout_missing')

    running_version = worker_res.get('wechat_version')
    if not validate_version_string(running_version):
        raise ValueError('layout_missing')

    # 2. 经 remote 读取候选
    cand_path = f"{layout_dir}/candidate-wechat-{running_version}.json"
    proc = remote_runner(host, f"cat {shlex.quote(cand_path)}")
    if proc.returncode != 0:
        raise ValueError(f'candidate_not_found: {cand_path}')

    if len(proc.stdout) > 256 * 1024:
        raise ValueError('layout_too_large')

    try:
        candidate_data = json.loads(proc.stdout.decode('utf-8'))
    except Exception:
        raise ValueError('layout_unreadable')

    # 3. 本地校验候选
    validate_layout_candidate(candidate_data)

    # 4. 版本核对
    cand_version = candidate_data.get('wechat_version')
    if cand_version != running_version:
        raise ValueError(f'version_mismatch: candidate is {cand_version}, running is {running_version}')

    # 5. 远端单条原子命令完成备份与原子替换
    active_path = f"{layout_dir}/wechat-{running_version}.json"
    timestamp = time.strftime('%Y%m%d%H%M%S')
    backup_path = f"{layout_dir}/wechat-{running_version}.json.bak-{timestamp}"

    cmd_atomic = (
        f"if [ -f {shlex.quote(active_path)} ]; then "
        f"cp {shlex.quote(active_path)} {shlex.quote(backup_path)} && chmod 600 {shlex.quote(backup_path)}; "
        f"fi && mv {shlex.quote(cand_path)} {shlex.quote(active_path)} && chmod 600 {shlex.quote(active_path)}"
    )
    proc_atomic = remote_runner(host, cmd_atomic)
    if proc_atomic.returncode != 0:
        raise RuntimeError('remote_accept_failed')

    return {
        'accepted': True,
        'version': running_version,
        'active_layout': active_path,
        'backup': backup_path
    }


def calibrate_command(args, remote_runner=None, worker_func=None):
    layout_dir = getattr(args, 'layout_dir', None) or REMOTE_LAYOUT_DIR
    config_path = Path(getattr(args, 'config', locations()[1]))
    config = load_config(config_path) if config_path.exists() else {}
    if loaded():
        sys.stderr.write('机器人正在运行：请先 control stop，再运行 calibrate\n')
        raise SystemExit(1)
    host = require_host(config)

    if getattr(args, 'accept', False):
        try:
            res = calibrate_accept(host=host, layout_dir=layout_dir, config=config, worker_func=worker_func, remote_runner=remote_runner)
            print(json.dumps(res, ensure_ascii=False))
            return
        except Exception as e:
            sys.stderr.write(f'calibrate accept failed: {e}\n')
            raise SystemExit(1)

    try:
        diffs, missing, candidate = run_calibrate(config=config, host=host, layout_dir=layout_dir, remote_runner=remote_runner, worker_func=worker_func)
    except Exception as e:
        sys.stderr.write(f'calibrate failed: {e}\n')
        raise SystemExit(1)

    version = candidate.get('wechat_version')
    if diffs:
        sys.stdout.write("Differences from current layout:\n")
        for d in diffs:
            sys.stdout.write(f"  {d['field']}: {d['old']} -> {d['new']}\n")
    else:
        sys.stdout.write("No layout differences detected.\n")

    if missing:
        sys.stdout.write("Missing measurements (preserved original values):\n")
        for m in missing:
            sys.stdout.write(f"  {m}\n")

    result = {
        'candidate_json': f"{layout_dir}/candidate-wechat-{version}.json",
        'candidate_png': f"{layout_dir}/candidate-wechat-{version}.png",
        'differences': diffs,
        'missing': missing
    }
    print(json.dumps(result, ensure_ascii=False))


if __name__=='__main__': main()
