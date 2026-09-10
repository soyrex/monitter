# Model catalog verification

Read-only verification used the Monitter Rust app-server reader: initialize, initialized,
config/read, then paginated model/list. It does not start a model turn or write configuration.

- Local Codex 0.154.0: 6 models returned; 5 advertise Fast.
- Generated app-server schema and local catalog identify Fast as `serviceTiers[].id = "priority"`.
  `additionalSpeedTiers: ["fast"]` is deprecated. Runner passes `service_tier="priority"` for
  Fast and `service_tier="default"` to explicitly disable an inherited Fast setting.
- SSH host `mira`: the saved Monitter host executable `/home/alex/.npm-global/bin/codex`
  (Codex 0.130.0) returned 5 models and advertises no Fast-capable model. Plain `codex` is absent
  from its noninteractive SSH PATH, so catalog verification must use the saved executable path.
  A null Fast setting preserves native behavior; any non-null Fast override is rejected when a
  catalog model does not advertise the capability.
- Local Codex 0.154.0 accepted read-only `service_tier="default"` parsing. Mira 0.130.0 rejects
  that value and lists `fast` or `flex`; Monitter does not guess an older semantic equivalent.
  No model turns, configuration writes, authentication changes, or native session changes occurred.
