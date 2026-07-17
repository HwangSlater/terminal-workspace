# ADR 0011: Notification Pipeline Stages Selection

## Context
Aggregating active messaging integrations (Slack, GitHub, Calendar) into a single terminal screen poses a threat of **Alert Fatigue**. If a build loop fails, hundreds of duplicate notifications might flood the UI. We need a structured filtering pipeline.

---

## Decision
We implement a multi-stage **Notification Pipeline**:
`Integration Event -> Rules Check -> Deduplication -> Rate Limiter -> Projection/DB Write`.

- **Deduplicator**: Collapses identical alerts in a sliding time window.
- **Rate Limiter**: Caps maximum visual overlay alerts pushed per second.

---

## Alternatives Considered

### Direct Event Injection to UI
- **Pros**: Zero latency.
- **Cons**: Severe UI flickering and keyboard input blockage if multiple alerts arrive simultaneously. (Rejected).

---

## Consequences
- **User Control**: Developers can configure rules to filter out low-priority alerts before they hit the screen buffer.
- **Stability**: Prevents terminal rendering loops from slowing down during high integration throughput.
