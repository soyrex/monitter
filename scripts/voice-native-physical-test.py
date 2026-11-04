#!/usr/bin/env python3
"""Bounded local-only physical duplex probe. Intentionally opens microphone on start.

Usage: python3 scripts/voice-native-physical-test.py [--playback-completion | --capture-soak-seconds 65] <helper> <models-directory>
It prints control metadata and transcript event counts only, never transcript text or PCM.
The helper is stopped/killed on every exit; capture has an independent kill timer.
"""
import argparse, json, os, pathlib, queue, subprocess, threading, time, uuid
parser=argparse.ArgumentParser(description='Bounded physical local voice probe')
modes=parser.add_mutually_exclusive_group()
modes.add_argument('--playback-completion', action='store_true', help='mute capture and test playback completion followed by active cancellation')
modes.add_argument('--push-to-talk', action='store_true', help='exercise bounded finish-input and reopen cycles without agent submission')
modes.add_argument('--capture-soak-seconds', type=int, choices=range(30,91), metavar='30..90', help='observe continuous capture without sending speech to an agent')
parser.add_argument('helper'); parser.add_argument('models')
args=parser.parse_args()
helper, models = (str(pathlib.Path(p).resolve()) for p in (args.helper,args.models))
env={k:os.environ[k] for k in ('HOME','TMPDIR','LANG') if k in os.environ}
env['PATH']='/usr/bin:/bin:/usr/sbin:/sbin'
diagnostics=not args.playback_completion and os.environ.get('MONITTER_VOICE_DIAGNOSTICS') == '1'
if diagnostics: env['MONITTER_VOICE_DIAGNOSTICS']='1'
p=subprocess.Popen([helper],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=None if diagnostics else subprocess.DEVNULL,text=True,bufsize=1,env=env)
q=queue.Queue(); timer=None; started=False; session,epoch=str(uuid.uuid4()),str(uuid.uuid4())
counts={'partial':0,'final':0,'activityEdges':0}
def reader():
    for line in p.stdout:
        try: event=json.loads(line)
        except ValueError: q.put({'type':'error','code':'invalidJSONOutput','_transport':True}); continue
        if not isinstance(event,dict): q.put({'type':'error','code':'invalidJSONOutput','_transport':True}); continue
        if event.get('sessionId') != session or event.get('epoch') != epoch: continue
        if event.get('type')=='transcript':
            counts['final' if event.get('isFinal') else 'partial']+=1
            event={k:v for k,v in event.items() if k!='text'}
        if event.get('type')=='inputActivity': counts['activityEdges']+=1
        q.put(event)
    q.put({'type':'error','code':'helperExited','_transport':True})
threading.Thread(target=reader,daemon=True).start()
def send(kind,**fields):
    request=str(uuid.uuid4()); obj={'type':kind,'sessionId':session,'epoch':epoch,'requestId':request,**fields}
    p.stdin.write(json.dumps(obj)+'\n'); p.stdin.flush(); return request
def check_error(e):
    if e.get('type')=='error':
        code=e.get('code','unknown')
        # startFailed is generated from permission or audio-engine setup, before capture.
        # Do not print recognition or synthesis errors, which may contain private text.
        detail=str(e.get('message','')) if code=='startFailed' else ''
        raise RuntimeError(f"helper error {code}: {detail}" if detail else f"helper error {code}")

def expect(kind,request=None,status=None,timeout=10,generation=None):
    deadline=time.monotonic()+timeout
    while True:
        left=deadline-time.monotonic()
        if left<=0: raise TimeoutError(f'{kind} {status}')
        try: e=q.get(timeout=left)
        except queue.Empty: raise TimeoutError(f'{kind} {status}') from None
        check_error(e)
        if e.get('type')==kind and (request is None or e.get('requestId')==request) and (status is None or e.get('status') in status) and (generation is None or e.get('generation')==generation):
            print(json.dumps({k:v for k,v in e.items() if k in ('type','status','voiceProcessing','generation','code','loadMs')}),flush=True)
            return e
try:
    r=send('load',modelsPath=models); expect('ready',r,{'loaded'},timeout=90)
    # Bound capture even if permission, inference, or cleanup stalls.
    timer=threading.Timer(args.capture_soak_seconds+30 if args.capture_soak_seconds else 45,lambda: p.kill() if p.poll() is None else None); timer.daemon=True; timer.start()
    r=send('start'); e=expect('ready',r,{'started'},timeout=20); started=True
    if e.get('voiceProcessing') is not True: raise RuntimeError('voice processing not enabled')
    if args.capture_soak_seconds:
        began=time.monotonic(); deadline=began+args.capture_soak_seconds
        while (left:=deadline-time.monotonic()) > 0:
            try: check_error(q.get(timeout=min(left,1)))
            except queue.Empty: pass
        r=send('mute',muted=True); expect('ready',r,{'muted'})
        print(json.dumps({'captureSoakSeconds':round(time.monotonic()-began,3)}),flush=True)
    elif args.push_to_talk:
        for _ in range(2):
            # Drain accepted microphone buffers before closing the utterance.
            time.sleep(0.5)
            r=send('finishInput'); expect('ready',r,{'inputFinished'},timeout=23)
            r=send('mute',muted=False); expect('ready',r,{'unmuted'},timeout=23)
        r=send('mute',muted=True); expect('ready',r,{'muted'})
    elif args.playback_completion:
        r=send('mute',muted=True); expect('ready',r,{'muted'})
        r=send('speak',generation=1,text='Hello. This is a local voice playback test.')
        expect('audioProgress',r,{'started'},generation=1)
        # Completion is asynchronous and carries a generation, not a request ID.
        expect('audioProgress',status={'completed'},generation=1,timeout=25)
        r=send('speak',generation=2,text='This longer test checks that playback can stop when requested. It contains several sentences. The remaining words should be cancelled before the spoken test finishes. This is a local test of the audio player.')
        expect('audioProgress',r,{'started'},generation=2)
        r=send('cancelSpeech',generation=2); expect('audioProgress',r,{'cancelled'},generation=2)
    else:
        r=send('speak',generation=1,text='Hello. This is a local voice test.')
        expect('audioProgress',r,{'started'},generation=1)
        r=send('pauseSpeech',generation=1); expect('audioProgress',r,{'paused','alreadyFinished'},generation=1)
        r=send('resumeSpeech',generation=1); expect('audioProgress',r,{'resumed','alreadyFinished'},generation=1)
        time.sleep(2)  # Allow up to two seconds of the fixed, non-private test phrase to play.
        r=send('cancelSpeech',generation=1); expect('audioProgress',r,{'cancelled','alreadyFinished'},generation=1)
        r=send('mute',muted=True); expect('ready',r,{'muted'})
        r=send('mute',muted=False); expect('ready',r,{'unmuted'},timeout=23)
    r=send('stop'); expect('ready',r,{'stopped'}); started=False
    print(json.dumps({'ok':True,'counts':counts}),flush=True)
finally:
    if started and p.poll() is None:
        try: send('stop')
        except (BrokenPipeError,OSError): pass
    try: p.stdin.close()
    except (BrokenPipeError,OSError): pass
    try: p.wait(timeout=3)
    except subprocess.TimeoutExpired: p.kill(); p.wait()
    if timer: timer.cancel()
