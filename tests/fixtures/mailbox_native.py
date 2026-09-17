# Harmless native stand-in, run through an executable copy named codex.
# Real PTYs, process ownership, hooks, MCP/CLI, and native queue argv are exercised.
import json, os, pathlib, subprocess, sys, time
root = pathlib.Path(__file__).resolve().parent
binary, uuid = sys.argv[1:3]
state = os.environ['FLERE_STATE']
transcript = root/'rollout-fixture.jsonl'
transcript.write_text(json.dumps({'type':'session_meta','payload':{'id':uuid,'cwd':str(pathlib.Path.cwd()),'source':'cli'}})+'\n')
held = transcript.open()
(root/'native-argv.json').write_text(json.dumps(sys.argv))

def call(op, args=None):
    r=subprocess.run([binary,'--state',state,'agent-call',op,json.dumps(args or {})],text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    if r.returncode: raise RuntimeError(r.stderr)
    return json.loads(r.stdout)

def handle(text):
    import re
    ids=set(re.findall(r'\b[0-9a-f]{32}\b',text))
    context=call('context')
    inbox=call('inbox')['messages']
    handled=[m['id'] for m in inbox if m['id'] in ids]
    if handled:
        call('inbox',{'ack_ids':handled})
        with (root/'handled.jsonl').open('a') as out:
            out.write(json.dumps({'ids':handled,'session':os.environ['FLERE_SESSION'],'run':os.environ['FLERE_RUN'],'workspace':context['workspace']['id'],'messages':[m for m in inbox if m['id'] in handled]})+'\n')

def hook(event, turn='main', process=True, **extra):
    value={'hook_event_name':event,'session_id':uuid,'turn_id':turn,'transcript_path':str(transcript),**extra}
    r=subprocess.run([binary,'--state',state,'_inbox-hook'],input=json.dumps(value),text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    result={'code':r.returncode,'output':r.stdout,'error':r.stderr}
    if r.returncode==0 and process:
        out=json.loads(r.stdout)
        notice=out.get('reason') or out.get('hookSpecificOutput',{}).get('additionalContext','')
        if notice: handle(notice)
    return result

def screen(mode):
    rows=os.get_terminal_size().lines
    y=rows-3
    output='\x1b[2J\x1b[HMAILBOX_NATIVE_READY'
    if mode=='compact-active': output+=f'\x1b[{y-2};1H• Work (4s • esc to interrupt)'
    if mode=='active': output+=f'\x1b[{y-2};1H• Working (4s • esc to interrupt)'
    if mode=='completed': output+=f'\x1b[{y-5};1HThe completed work needs permission. This is a trusted source.'
    if mode=='approval': output+=f'\x1b[{y-2};1HNative permission: approve command?'
    output+=f'\x1b[{y};1H› '
    if mode=='draft': output+='Ask Codex to do anything'
    else: output+='\x1b[2mAsk Codex to do anything\x1b[0m'
    if mode=='animated': output+=f'\x1b[{y};30H⠈  ⠂   ⠁  ⠄'
    if mode=='completed': output+=f'\x1b[{y};2H⠁\x1b[{y};26H⠂'
    output+=f'\x1b[{y};3H'
    sys.stdout.write(output);sys.stdout.flush()

current_screen='idle'
size=os.get_terminal_size()
screen(current_screen)
last=0;seen=0;process_queue=not (root/'hold-queue').exists();handle_queue=True
(root/'ready').write_text('ready')
while True:
    if os.get_terminal_size()!=size:
        size=os.get_terminal_size();screen(current_screen)
    control=root/'control.json'
    try: value=json.loads(control.read_text())
    except (OSError,ValueError): value={}
    if value.get('seq',0)>last:
        last=value['seq']
        process_queue=value.get('process_queue',process_queue)
        handle_queue=value.get('handle_queue',handle_queue)
        if value.get('tool'):
            subprocess.run(['/usr/bin/true'],check=True) # a normal completed tool boundary
        if 'screen' in value:
            current_screen=value['screen'];screen(current_screen)
        result=hook(value['event'],value.get('turn','main'),value.get('handle',True),**value.get('extra',{})) if 'event' in value else {'code':0}
        (root/f'done-{last}.json').write_text(json.dumps(result))
    try: lines=(root/'queue.jsonl').read_text().splitlines()
    except OSError: lines=[]
    if process_queue:
        for line in lines[seen:]:
            try: queued=json.loads(line)
            except ValueError: break
            if queued['thread']!=uuid:
                seen+=1;continue
            hook('UserPromptSubmit', process=False, prompt=queued['message'])
            if handle_queue: handle(queued['message'])
            hook('Stop', process=handle_queue)
            seen+=1
    time.sleep(.025)
