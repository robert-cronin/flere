"""Harmless Claude CLI stand-in. No model/network calls except an optional local fixture.
Exercises real Flere MCP and command hooks from the owned foreground child.
"""
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path(os.environ['HOME'])
args = sys.argv[1:]
if any(a in args for a in ['--version', '--print', '-p', 'mcp', 'doctor']):
    (root / 'passthrough.json').write_text(json.dumps({'args': args}))
    print('CLAUDE_PASSTHROUGH', flush=True)
    sys.exit(7)
settings_path = Path(args[max(i for i, a in enumerate(args) if a == '--settings') + 1])
settings = json.loads(settings_path.read_text())
os.environ.update(settings.get('env', {}))
servers = {}
for source in args[args.index('--mcp-config') + 1:]:
    if source.startswith('-'):
        break
    config = json.loads(source if source.startswith('{') else Path(source).read_text())
    servers.update(config['mcpServers'])
mcp = servers['flere']
uuid = args[args.index('--resume') + 1] if '--resume' in args and args.index('--resume') + 1 < len(args) else '11111111-2222-3333-4444-555555555555'
run = os.environ['FLERE_RUN']

def hook(name, extra=None):
    event = {'hook_event_name': name, 'session_id': uuid,
             'transcript_path': str(root / '.claude/projects/fixture' / (uuid + '.jsonl'))}
    event.update(extra or {})
    command = settings['hooks'][name][0]['hooks'][0]['command']
    child = subprocess.run(command, shell=True, input=json.dumps(event), capture_output=True, text=True, timeout=10)
    return {'code': child.returncode, 'out': json.loads(child.stdout or '{}'), 'error': child.stderr}

def mcp_call(name, arguments=None):
    request = {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
               'params': {'name': name, 'arguments': arguments or {}}}
    child = subprocess.run([mcp['command']] + mcp['args'], input=json.dumps(request) + '\n',
                           capture_output=True, text=True, timeout=10)
    return {'code': child.returncode, 'response': json.loads(child.stdout or '{}')}

receipt = {'args': args, 'settings': settings, 'mcp_servers': servers, 'settings_path': str(settings_path),
           'start': hook('SessionStart'), 'context': mcp_call('get_context')}
(root / ('launch-' + run + '.json')).write_text(json.dumps(receipt))
print('CLAUDE_READY', flush=True)
for line in sys.stdin:
    request = json.loads(line)
    if request.get('exit'):
        hook('SessionEnd')
        break
    if 'uuid' in request:
        uuid = request['uuid']
    if 'mcp' in request:
        response = mcp_call(request['mcp'], request.get('arguments'))
    else:
        response = hook(request['event'], request.get('extra'))
    (root / ('reply-' + str(request['id']) + '.json')).write_text(json.dumps(response))
    print('CLAUDE_REPLY_' + str(request['id']), flush=True)
