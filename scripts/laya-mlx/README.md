# Local Laya MLX adapter on Linux

This optional adapter runs the English Laya SystemOne model on the Apple GPU using an already installed MLX Omarchy runtime and a converted local model. The tested combination is MLX Omarchy 0.7.38, FP32 and Apple M2 Linux; other versions and devices require validation. It does not download a model or runtime. GPU initialization fails closed rather than falling back to CPU.

Install using absolute paths without spaces or shell metacharacters:

```sh
python3 scripts/laya-mlx/install.py \
  --python /path/to/venv/bin/python \
  --serve-dir /path/to/omarchy-mlx/serve \
  --model-dir /path/to/converted/laya
```

The installer writes the adapter, local `runtime.json`, and a systemd user unit. It refuses to overwrite an existing installation or unit. It does not start the service unless `--enable` is supplied. To manage it later, run `systemctl --user daemon-reload` followed by `systemctl --user enable --now monitter-laya-mlx.service`; use `systemctl --user status` or `stop` with that unit name to inspect or stop it. Port 8080 must be free; the owned MLX worker uses loopback port 8082. Set `LAYA_API_KEY` in the service environment when authentication is needed.

For Monitter's on-demand launcher, set `LAYA_CLI_PATH` to the installed `laya-cli`, `LAYA_MODEL_DIR` to the converted model path, and `LAYA_RUNTIME_BACKEND=vulkan`. The wrapper accepts those launcher flags while using MLX GPU FP32. The adapter supports `/health`, `/v1/models`, and `/v1/systemone`, translates answer metadata, enforces native strict token budgets, and limits requests to eight questions. It keeps inference bounded and serial, and its worker does not download files.

After startup, check `curl --fail http://127.0.0.1:8080/health`; its backend
must be `MLX GPU FP32`. Run the adapter's focused tests with:

```sh
python3 -m unittest discover -s scripts/laya-mlx -v
```

The integration was checked against 250 fixed requests: 220 matching successful
responses and 30 matching strict rejections. Mixed-question batches of two,
four and eight were also compared. These checks cover the supplied English
model and FP32 runtime; they do not qualify other models or precision modes.
