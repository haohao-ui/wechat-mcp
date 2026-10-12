"""Run a persistent CRM/inbox workflow through the actual MCP executable."""
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time

class Client:
    def __init__(self, binary, directory):
        env = dict(os.environ)
        env['MESSAGE_GATEWAY_DB'] = str(directory / 'service.sqlite3')
        env['MESSAGE_GATEWAY_CONFIG'] = str(directory / 'accounts.json')
        self.process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, env=env)
        self.responses = queue.Queue()
        self.counter = 0
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()
        self.request('initialize', {'protocolVersion':'2025-06-18','capabilities':{},'clientInfo':{'name':'crm-test','version':'1'}})
        self.process.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n')
        self.process.stdin.flush()
        self.schemas = {tool['name']: tool['inputSchema'] for tool in self.request('tools/list', {})['result']['tools']}

    def _read(self):
        for line in self.process.stdout:
            self.responses.put(json.loads(line))

    def request(self, method, params):
        self.counter += 1
        self.process.stdin.write(json.dumps({'jsonrpc':'2.0','id':self.counter,'method':method,'params':params})+'\n')
        self.process.stdin.flush()
        response = self.responses.get(timeout=5)
        assert response['id'] == self.counter, response
        assert 'error' not in response, response
        return response

    def tool(self, name, arguments, expect_error=False):
        arguments = {'platform':'wechat','account_id':'local',**arguments}
        schema = self.schemas[name]
        assert set(schema.get('required',[])).issubset(arguments), (name,schema,arguments)
        assert set(arguments).issubset(schema['properties']), (name,arguments)
        result = self.request('tools/call', {'name':name,'arguments':arguments})['result']
        assert result['isError'] == expect_error, result
        if not expect_error:
            return result['structuredContent']['data']
        return result

    def close(self):
        self.process.stdin.close()
        assert self.process.wait(timeout=5) == 0
        self.thread.join(timeout=1)

    def abort(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait()


def main():
    binary = Path(sys.argv[1] if len(sys.argv)>1 else 'target/release/wechat-mcp').resolve()
    with tempfile.TemporaryDirectory(prefix='crm-stdio-test-') as temp:
        directory = Path(temp)
        (directory/'accounts.json').write_text(json.dumps({'accounts':[{'platform':'wechat','account_id':'local','adapter':'wechat_desktop'}]}))
        client = Client(binary,directory)
        try:
            customer_request = {'idempotency_key':'customer-1','display_name':'Simulated customer','recipient':'Simulated customer','tags':['priority']}
            customer = client.tool('create_customer', customer_request)
            assert client.tool('create_customer', customer_request)['customer_id'] == customer['customer_id']
            customer_id = customer['customer_id']
            client.tool('update_customer', {'customer_id':customer_id,'expected_version':1,'status':'active'})
            client.tool('update_customer', {'customer_id':customer_id,'expected_version':1,'notes':'stale'}, expect_error=True)
            event_request = {'customer_id':customer_id,'idempotency_key':'event-1','text':'Simulated support interaction','direction':'inbound','enqueue':True}
            event = client.tool('record_interaction',event_request)
            assert event['verified'] is False
            assert client.tool('record_interaction',event_request)['event_id'] == event['event_id']
            waiting = client.tool('wait_for_events',{'timeout_seconds':0})
            assert waiting['status'] == 'available'
            claimed = client.tool('claim_events',{'consumer':'service-test'})['events']
            assert len(claimed) == 1
            lease = {'event_id':event['event_id'],'consumer':'service-test','lease_token':claimed[0]['lease_token']}
            client.tool('renew_event_lease',{**lease,'lease_seconds':120})
            client.tool('record_customer_analysis',{'idempotency_key':'analysis-1','customer_id':customer_id,'summary':'Agent inference from simulated data','evidence_event_ids':[event['event_id']]})
            followup = {'idempotency_key':'followup-1','customer_id':customer_id,'title':'Follow up simulated issue','due_at':int(time.time())-1,'owner':'service-test'}
            task = client.tool('create_followup',followup)
            assert client.tool('create_followup',followup)['task_id'] == task['task_id']
            client.tool('complete_followup',{'task_id':task['task_id'],'outcome':'Simulated resolution'})
            client.tool('ack_event',lease)
            client.tool('ack_event',lease)
            assert client.tool('wait_for_events',{'timeout_seconds':0})['status'] == 'timeout'
            metrics = client.tool('get_service_metrics',{})
            assert metrics['events']['agent_records'] == 1
            assert metrics['events']['verified_incoming'] == 0
            assert metrics['delivery_rate'] is None and metrics['response_time'] is None
            client.close()
        finally:
            client.abort()
        backup = directory / 'backup.sqlite3'
        checked = subprocess.run([sys.executable,str(Path(__file__).resolve().parents[1]/'scripts/backup_db.py'),'--source',str(directory/'service.sqlite3'),'--output',str(backup)],capture_output=True,text=True)
        assert checked.returncode == 0, checked.stderr
        duplicate = subprocess.run([sys.executable,str(Path(__file__).resolve().parents[1]/'scripts/backup_db.py'),'--source',str(directory/'service.sqlite3'),'--output',str(backup)],capture_output=True,text=True)
        assert duplicate.returncode != 0
        # Restore into the configured path with a complete SQLite backup, never
        # copy an active main database while its WAL might hold committed data.
        import sqlite3
        with sqlite3.connect(backup) as reader:
            with sqlite3.connect(directory/'service.sqlite3') as writer:
                reader.backup(writer)
        writer.close()
        reader.close()
        client = Client(binary,directory)
        try:
            customer = client.tool('get_customer',{'customer_id':customer_id})
            assert customer['customer']['status'] == 'active'
            assert customer['events'][0]['state'] == 'acknowledged'
            assert len(client.tool('get_customer_analyses',{'customer_id':customer_id})['analyses']) == 1
            assert len(client.tool('list_followups',{'status':'done'})['tasks']) == 1
            client.close()
        finally:
            client.abort()
    print('PASS: persistent customer/inbox/followup/analysis workflow and restart through real MCP; all data simulated, no desktop operations.')

if __name__ == '__main__':
    main()
