"""Install and inspect the current user's launchd bot service. Never edits the remote GUI bundle."""
import argparse
import json
import os
from pathlib import Path
import plistlib
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
    sub.add_parser('history-http').add_argument('command',choices=['install','start','stop','status'])
    args=parser.parse_args(argv)
    base,config_path,plist_path=locations()
    if args.action in ('migrate-history','history-status','merge-contact','unmerge-contact','history-backup'):
        return history_command(args)
    if args.action=='history-http':
        return history_http(args.command,config_path,base)
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
        print(json.dumps({'launchd_loaded':loaded(),'health':health,'stats':stats,'config':str(config_path)},ensure_ascii=False))


if __name__=='__main__': main()
