# Status: ready-for-agent

## Seams for Ticket 01

Confirmed ladder: **Probe → CLI → …**. Ticket 01 tests at the **Probe process I/O** seam only.

External behaviour asserted:
1. `envbox-probe` stdout contains independent section headers `GEO`, `LOCALE`, `LANGUAGE`, `TIMEZONE`, `DNS`, `ENV` and stable probe key names.
2. `envbox-probe --spawn-child` produces parent and child snapshot blocks (two `===` sections).
3. Host system config is not modified (probe is read-only by construction).
