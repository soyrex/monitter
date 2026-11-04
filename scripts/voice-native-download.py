#!/usr/bin/env python3
"""Acquire only pinned, public, local voice assets. Never uploads audio."""
import argparse, hashlib, json, os, pathlib, urllib.request, zipfile, sys
def model_directory(override=None):
    value = override if override is not None else os.environ.get('MONITTER_VOICE_MODELS')
    root = pathlib.Path(value) if value is not None else pathlib.Path.home() / 'Library/Caches/Monitter/VoiceModels'
    if not root.is_absolute():
        raise ValueError('The voice model directory must be an absolute path.')
    return root
REV = '40a23f4c0b333aa17ad8c0f2ea47ec2347f2f355'
REPO = 'FluidInference/parakeet-realtime-eou-120m-coreml'
UA = {'User-Agent': 'Monitter-native-voice-prototype/1'}
def get(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60)
def fetch(url, path, size, digest, kind='sha256'):
    if size > 250_000_000: raise ValueError('asset exceeds per-file bound')
    def verified(p):
        if not p.is_file() or p.stat().st_size != size: return False
        h = hashlib.sha256() if kind == 'sha256' else hashlib.sha1(f'blob {size}\0'.encode())
        with p.open('rb') as f:
            for chunk in iter(lambda: f.read(1024*1024), b''): h.update(chunk)
        return h.hexdigest() == digest
    if verified(path): return
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix(path.suffix + '.part')
    try:
        with get(url) as r, temp.open('wb') as f:
            total=0
            while chunk := r.read(1024*1024):
                total += len(chunk)
                if total > size: raise ValueError('asset larger than declared')
                f.write(chunk)
        if not verified(temp): raise ValueError(f'length or digest mismatch: {path}')
        temp.replace(path)
    finally:
        temp.unlink(missing_ok=True)
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--models-dir', help='Absolute model directory (default: MONITTER_VOICE_MODELS or ~/Library/Caches/Monitter/VoiceModels)')
    args = parser.parse_args()
    ROOT = model_directory(args.models_dir)
    ROOT.mkdir(parents=True, exist_ok=True)
    with get(f'https://huggingface.co/api/models/{REPO}/revision/{REV}?blobs=true') as r: info=json.load(r)
    if info['sha'] != REV: raise ValueError('revision mismatch')
    files = [x for x in info['siblings'] if x['rfilename'].startswith('320ms/') and ('.mlmodelc/' in x['rfilename'] or x['rfilename'] in ('320ms/vocab.json','320ms/streaming_encoder_metadata.json'))]
    if len(files) != 17 or sum(x['size'] for x in files) > 230_000_000: raise ValueError('unexpected manifest')
    for x in files:
        name=x['rfilename']; meta=x.get('lfs'); digest=meta['sha256'] if meta else x['blobId']
        fetch(f'https://huggingface.co/{REPO}/resolve/{REV}/{name}', ROOT/'parakeet-eou-320'/name.removeprefix('320ms/'), x['size'], digest, 'sha256' if meta else 'git')
    archive=ROOT/'phonon-31m-en-coreml-v0.6.0.zip'
    fetch('https://github.com/gradium-ai/xn-ptts/releases/download/v0.6.0/phonon-31m-en-coreml.zip', archive, 137300815, '2869326ab074f55ed5b644e68b1befef69dc51545b73296d49f5b4dab14c4218')
    dest=ROOT/'phonon-31m-en-v0.6.0'
    if not (dest/'.complete').exists():
        with zipfile.ZipFile(archive) as z:
            if sum(i.file_size for i in z.infolist()) > 400_000_000: raise ValueError('archive too large')
            for i in z.infolist():
                p=pathlib.PurePosixPath(i.filename)
                if p.is_absolute() or '..' in p.parts or (i.external_attr >> 16) & 0o170000 == 0o120000: raise ValueError('unsafe archive')
            z.extractall(dest)
        (dest/'.complete').write_text('sha256 2869326ab074f55ed5b644e68b1befef69dc51545b73296d49f5b4dab14c4218\n')
    print('Assets verified:', ROOT)

if __name__ == '__main__':
    main()
