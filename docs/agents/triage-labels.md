# Triage Labels

This repository uses the canonical Triage state names as the `Status:` value in local Markdown issues.

| Canonical state | Local Markdown value | Meaning |
| --- | --- | --- |
| `needs-triage` | `needs-triage` | Maintainer needs to evaluate the task |
| `needs-info` | `needs-info` | Waiting for missing information |
| `ready-for-agent` | `ready-for-agent` | Fully specified and ready for an Agent |
| `ready-for-human` | `ready-for-human` | Needs human implementation or decision |
| `wontfix` | `wontfix` | Will not be actioned |

Each issue should carry exactly one state in its `Status:` line. Enhancement and bug category remain separate from the state.
