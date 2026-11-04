#!/usr/bin/env python3
"""No microphone or speaker; verifies child JSONL and cancellation with local models."""
import json, os, pathlib, queue, subprocess, sys, threading, time, uuid
root = pathlib.Path(__file__).resolve().parents[1]
exe = pathlib.Path(sys.argv[1]).resolve()
env = dict(os.environ, MONITTER_VOICE_TEST_SINK='1')
p = subprocess.Popen([str(exe)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1, env=env)
q = queue.Queue()
def reader():
    for line in p.stdout:
        try: q.put(json.loads(line))
        except ValueError: q.put({'type':'invalid-output', 'line':line})
threading.Thread(target=reader,daemon=True).start()
session, epoch = str(uuid.uuid4()), str(uuid.uuid4())
def send(kind, **fields):
    obj={'type':kind,'sessionId':session,'epoch':epoch,'requestId':str(uuid.uuid4()),**fields}
    p.stdin.write(json.dumps(obj)+'\n'); p.stdin.flush(); return obj['requestId']
def expect(kind, predicate=lambda x: True, timeout=120):
    end=time.monotonic()+timeout
    while time.monotonic()<end:
        e=q.get(timeout=max(0.01,end-time.monotonic()))
        if e.get('type') in ('error','invalid-output'): raise RuntimeError(str(e))
        if e.get('type') == kind and predicate(e): return e
    raise TimeoutError(kind)
try:
    send('load',modelsPath=str(root/'artifacts/local-voice-models'))
    loaded=expect('ready',lambda e:e.get('status')=='loaded')
    send('start',testOnlyNoAudio=True)
    ready=expect('ready',lambda e:e.get('status')=='started')
    assert ready['voiceProcessing'] is False and ready['testOnlyNoAudio'] is True
    send('speak',generation=1,text='Hello there.')
    expect('audioProgress',lambda e:e.get('generation')==1 and e.get('status')=='started')
    expect('audioProgress',lambda e:e.get('generation')==1 and e.get('status')=='generated')
    send('cancelSpeech',generation=1)
    expect('audioProgress',lambda e:e.get('generation')==1 and e.get('status')=='alreadyFinished')
    send('speak',generation=2,text='This is a local voice test.')
    expect('audioProgress',lambda e:e.get('generation')==2 and e.get('status')=='started')
    send('cancelSpeech',generation=1)
    expect('audioProgress',lambda e:e.get('generation')==1 and e.get('status')=='alreadyFinished')
    expect('audioProgress',lambda e:e.get('generation')==2 and e.get('status')=='generated')
    send('speak',generation=3,text='This is a test sentence. '*100)
    expect('audioProgress',lambda e:e.get('generation')==3 and e.get('status')=='started')
    send('pauseSpeech',generation=3)
    expect('audioProgress',lambda e:e.get('generation')==3 and e.get('status')=='paused')
    send('resumeSpeech',generation=3)
    expect('audioProgress',lambda e:e.get('generation')==3 and e.get('status')=='resumed')
    send('cancelSpeech',generation=3)
    expect('audioProgress',lambda e:e.get('generation')==3 and e.get('status')=='cancelled')
    send('stop'); expect('ready',lambda e:e.get('status')=='stopped')
    print(json.dumps({'ok':True,'modelLoadMs':loaded['loadMs'],'testOnlyNoAudio':True}))
finally:
    p.stdin.close()
    try: p.wait(timeout=10)
    except subprocess.TimeoutExpired: p.kill(); p.wait()
    if p.returncode: print('helper stderr:',p.stderr.read()[-3000:],file=sys.stderr)
