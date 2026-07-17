# ADR 0013: Rule Engine Integration

## Context
Developers need fine-grained control over which notifications prompt alarms. Hardcoding user-alert filters inside the codebase violates OCP and results in high recompilation maintenance cost.

---

## Decision
We integrate an in-process, lightweight **Rule Engine** executing user-defined conditional logic rules written in TOML/JSON.
- **Rule Parser**: Evaluates simple boolean expressions (e.g. `event.sender == "@lead"`) against the incoming Event entity.
- **Actions**: Matches alter notification priorities (`set_priority("High")`) or suppress notifications (`suppress()`).

---

## Alternatives Considered

### Lua Script-based Filters
- **Pros**: Turing-complete, high capability.
- **Cons**: Substantial memory overhead, requires binding Lua to every event lifecycle step, slower performance. (Rejected).

---

## Consequences
- **Static Configuration**: Rules can be written directly to `config.toml`, letting users modify alarm thresholds without code changes.
- **Low Performance Overhead**: AST parsing of simple logic strings runs in microseconds, maintaining low TUI rendering latencies.
