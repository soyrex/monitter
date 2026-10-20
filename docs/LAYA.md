# Local Laya decisions

Enable **Jev decision tools**, select **Decision backend: Laya local**, and save the agent.
On the next decision call, Monitter reuses a healthy Laya server on `127.0.0.1:8080`.
If no listener exists, it starts the installed Laya CLI in the background and waits
up to 60 seconds for readiness. No terminal window or manual server command is needed.
Monitter never downloads models, starts a paid service, or falls back to TypeSafe.

The default Apple Silicon installation is `~/code/laya.cpp/build-coreml/bin/laya-cli`
with model directory `~/code/laya.cpp/models/laya`, using Core ML. Other platforms
use CPU by default and should configure their installed CLI path.

Optional environment variables on the Monitter backend process:

- `LAYA_CLI_PATH`: absolute installed executable path.
- `LAYA_MODEL_DIR`: absolute checkpoint directory containing `rl_agent_config.json`.
- `LAYA_RUNTIME_BACKEND`: `coreml`, `cpu`, `cuda`, or `vulkan`.
- `LAYA_DEFAULT_MODEL`: optional model override; by default use the actual model
  reported by the server's health response.
- `LAYA_API_KEY`: optional authentication, also supported through Environment & Secrets.

Startup and error output goes to `~/.monitter/laya-service/server.log` (private directory).
This does not add request-body logging to Laya. Monitter stops only its own child
process after ten minutes without decision calls; the next call starts it again.
Externally started servers are reused and never stopped. An occupied port with an
invalid health response is reported as unavailable rather than replaced.

Backend source changes require a backend rebuild and activation. Restarting only
the GUI does not activate backend changes.
