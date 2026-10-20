# Restarting Monitter on macOS

Use the Monitter menu-bar item → **Restart Monitter and Backend…**.
This saves and closes windows, replaces the persistent backend, waits for a
fresh authenticated backend connection, then opens the same GUI and profile.

**Wait for Agents** pauses new turns and waits for existing turns to finish.
The menu shows the running-agent count; Active Agents lists their names.
While waiting, Cancel Pending Restart restores normal admission. Choose
Restart again and Force Restart Now to interrupt agents instead. Force records
an exact continuation journal before cancelling their owned transports; the
new backend applies the existing recovery rules. Unconfirmed actions can
still require review.

Restart preparation, GUI shutdown, old-process exit, backend readiness and
GUI launch have explicit deadlines. A blocked stage reports an error instead
of waiting forever. A window that cannot close safely keeps the old backend
alive. No GUI or backend is force-killed by the coordinator. An error after
replacement starts leaves its process and recovery journal available for
inspection. Details are private `restart-*.receipt.json` and `restart-*.log`
files inside the selected GPUI profile; failures also show a native alert.

The replacement is resolved beside the configured GUI, so installing a new
bundle while an older process runs does not reopen a renamed rollback copy.
Only GUI processes belonging to that exact application and profile are closed;
other previews and profiles remain open. Closing the GUI normally still leaves
the persistent backend and agents running. Mobile/LAN clients have no restart
command.
