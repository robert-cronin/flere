# Invoked by the copied Python executable as: codex queue --thread UUID --message TEXT.
import json, os, pathlib, sys, time
root=pathlib.Path.cwd()
args=sys.argv[1:]
assert len(args)==4 and args[0]=='--thread' and args[2]=='--message',args
value={'thread':args[1],'message':args[3],'env':{k:os.environ.get(k) for k in ['HOME','CODEX_HOME','XDG_CONFIG_HOME','FLERE_RUN','FLERE_SESSION','CODEX_THREAD_ID','TMPDIR']}}
with (root/'queue.jsonl').open('a') as out: out.write(json.dumps(value)+'\n')
mode=(root/'queue-mode').read_text().strip() if (root/'queue-mode').exists() else ''
if mode=='hang': time.sleep(60)
if mode=='fail': sys.exit(1)
