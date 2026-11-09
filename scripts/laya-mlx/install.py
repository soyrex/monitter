#!/usr/bin/env python3
"""Install the local adapter using an already-installed runtime and model."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

FILES = ('adapter.py', 'strict_sequence.py', 'worker.py', 'laya-cli')


def checked_path(value):
    p = Path(os.path.abspath(Path(value).expanduser()))
    # systemd ExecStart has its own quoting and specifier rules. Keep paths
    # unambiguous rather than silently accepting shell metacharacters.
    if not re.fullmatch(r'/[A-Za-z0-9_./-]+', str(p)):
        raise ValueError(f'Unsupported path characters (including spaces): {p}')
    return p


def install(python, serve, model, destination, unit_dir):
    python, serve, model, destination, unit_dir = map(checked_path, (python, serve, model, destination, unit_dir))
    if not python.is_file() or not os.access(python, os.X_OK): raise ValueError('Python executable is missing')
    if not (serve/'mlx_omarchy_laya/server.py').is_file(): raise ValueError('MLX serve source is missing')
    if not (model/'manifest.json').is_file() or not (model/'model.safetensors').is_file(): raise ValueError('Converted model is missing')
    unit = unit_dir/'monitter-laya-mlx.service'
    if unit.exists() or any((destination/name).exists() for name in (*FILES, 'runtime.json')):
        raise ValueError('Destination or unit already exists; preserve it and choose a new location')
    created = []
    made_destination = not destination.exists()
    made_unit_dir = not unit_dir.exists()
    try:
        destination.mkdir(parents=True, exist_ok=True, mode=0o700)
        unit_dir.mkdir(parents=True, exist_ok=True)
        source = Path(__file__).resolve().parent
        def write_new(path, data, mode=0o644):
            # Exclusive creation never overwrites a file created by a concurrent install.
            with path.open('xb') as out:
                created.append(path)
                out.write(data)
            path.chmod(mode)
        for name in FILES:
            write_new(destination/name, (source/name).read_bytes(), 0o755 if name == 'laya-cli' else 0o644)
        config = json.dumps({'python':str(python),'serveDir':str(serve),'modelDir':str(model)}, indent=2)+'\n'
        write_new(destination/'runtime.json', config.encode(), 0o600)
        content = ('[Unit]\nDescription=Monitter local Laya MLX adapter\n\n[Service]\nType=simple\nExecStart='
                   +str(destination/'laya-cli')+' --server --vulkan --model '+str(model)
                   +'\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n')
        write_new(unit, content.encode())
    except BaseException:
        for path in reversed(created):
            try: path.unlink()
            except OSError: pass
        for directory, made in ((unit_dir, made_unit_dir), (destination, made_destination)):
            if made:
                try: directory.rmdir()
                except OSError: pass
        raise
    return unit


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--python', required=True); p.add_argument('--serve-dir', required=True); p.add_argument('--model-dir', required=True)
    p.add_argument('--install-dir', default=str(Path.home()/'.local/share/monitter-laya-mlx'))
    p.add_argument('--enable', action='store_true', help='Enable and start the installed user service')
    args=p.parse_args()
    unit_dir=Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home()/'.config')))/'systemd/user'
    try: unit=install(args.python,args.serve_dir,args.model_dir,args.install_dir,unit_dir)
    except (ValueError,OSError) as exc: p.error(str(exc))
    print(f'Installed {unit}; no downloads performed.')
    if args.enable:
        subprocess.run(['systemctl','--user','daemon-reload'],check=True)
        subprocess.run(['systemctl','--user','enable','--now',unit.name],check=True)
    else:
        print(f'To start: systemctl --user daemon-reload; systemctl --user enable --now {unit.name}')

if __name__=='__main__': main()
