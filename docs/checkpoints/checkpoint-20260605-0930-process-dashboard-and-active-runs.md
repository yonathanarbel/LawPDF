# Active Runs and Dashboard Patch - 2026-06-05 09:30

Dashboard:
- Restarted local dashboard on `http://127.0.0.1:8765/`.
- Active dashboard PID after restart: `24400`.
- Existing Cloudflare tunnel remains active: `cloudflared --url http://127.0.0.1:8765`.
- Public URL remains the existing tunnel URL.
- Password remains `Jcucmhe123`.

Dashboard code change:
- `tools\progress_dashboard.py` now includes Grok/header-footer and queued heading processes in process status.
- Command-line truncation now happens inside the PowerShell query before JSON serialization, so long `grok -p ...` prompts do not make `/api/status` time out.

Verified:
- `python -m py_compile tools\progress_dashboard.py`
- `/api/status` with Basic auth returned successfully.
- It showed:
  - Grok loop parent and workers.
  - Two active `grok.exe` CLI calls.
  - Queued heading page/caps/fragment trainer parent.
  - Active heading trainer from the Chandra loop.

Active processes observed around 09:28:
- `25348`: active `layout_role_training.py --binary-role heading` spawned by the Chandra loop.
- `22716`: waiting resource-gated heading trainer launcher for `layout-heading-pagecaps-earlymeta-allaccum-20260605-0920-candidate`.
- `33128`: Grok/header-footer specialist parent.
- `9492`: Grok label loop.
- `35640`, `19444`: Grok worker Python processes.
- `30280`, `26848`: active `grok.exe` labeling calls.

Resource state:
- Free RAM was still too low for another heavy job.
- The resource-gated heading launcher is correctly waiting for:
  - no active `layout_role_training.py`
  - no active Grok/header-footer run
  - free RAM >= 5 GB

Important note:
- The Chandra loop started a new heading trainer after the page/all-caps and fragment feature edits. It is writing to:
  - `profile-models\layout-heading-chandra-structure-disputes-heading-body-vllm-20260605-cycle002-candidate`
- That path name is old, but the process should be using the current edited feature code because it launched after the edits.
