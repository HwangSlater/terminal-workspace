# Integration Adapter State Machine

`ConnectionStatus` transitions (`docs/04-extensions/integration-contract.md`) for a polling-based adapter (Slack, and any future adapter built the same way — see `step6.md` for why polling was chosen over a persistent connection for Phase 6).

```text
                 no credential found
   ┌────────────────────────────────────────┐
   │                                         ▼
   │                                  Disconnected
   │                                         │
   │                              credential found,
   │                              first poll cycle runs
   │                                         │
   │                                         ▼
   │                                    Connected ◄──────────┐
   │                                    │      │             │
   │                          1-4 consecutive   successful   │
   │                          poll failures     poll cycle   │
   │                                    │      └─────────────┘
   │                                    ▼
   │                               Reconnecting
   │                                    │      ▲
   │                          5th-9th consecutive  successful
   │                          poll failure         poll cycle
   │                                    │      └──────────────┐
   │                                    │ 10th consecutive     │
   │                                    │ poll failure         │
   │                                    ▼                      │
   │                                Failed(reason) ────────────┘
   │                          (SystemAlert Event raised once,   successful poll
   │                           on the transition into Failed)   cycle resets
   └───────────────────────────────────────────────────────────┘ to Connected
```

Notes:
- There is no `Connecting` state in practice for a polling adapter — each cycle either succeeds (→ `Connected`) or fails (→ counter above), with no separate "establishing" phase. The `Connecting` variant exists on the trait for a future persistent-connection adapter (e.g. a Socket Mode upgrade) and is simply unused by `SlackAdapter`.
- The failure counter (1-4 / 5-9 / 10+) and the "skip a 429'd cycle without counting it as a failure" rule are specified in `integration-contract.md` §2.1-§2.2, not duplicated here.
- `Disconnected` is also the terminal (non-error) state when no credential was ever configured — it is not exclusively a failure state. See `integration-contract.md` §2.3.
